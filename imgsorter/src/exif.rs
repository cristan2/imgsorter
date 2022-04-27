use std::fs::{DirEntry, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use chrono::NaiveDateTime;
use exif::{Error, Exif, In, Tag};
use rexif::{ExifResult, ExifTag};

use crate::config::*;
use crate::utils::*;

const REXIF_DATE_FORMAT: &str = "%Y:%m:%d %H:%M:%S";
const KAMADAK_EXIF_DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Selected EXIF Data for a [[SupportedFile]]
/// Currently includes only the image date and camera model
#[derive(Debug)]
pub struct ExifDateDevice {
    pub date: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
}

impl ExifDateDevice {
    pub fn new() -> ExifDateDevice {
        ExifDateDevice {
            date: None,
            camera_make: None,
            camera_model: None,
        }
    }

    // Compose the device name based on the device make and model
    // If include_make is false, just return the model
    // Otherwise, make return a composite of "make model",
    // unless the model already starts with the make name,
    // e.g. "HUAWEI HUAWEI CAN-L11" should return "HUAWEI CAN-L11"
    pub fn get_device_name(&self, include_make: bool) -> Option<String> {
        self.camera_model
            .as_ref()
            .map(|camera_model| {
                if include_make {
                    self.camera_make.as_ref().map_or_else(
                        || camera_model.clone(),
                    |camera_make| {
                        // Only include the camera make if the model doesn't already contain it
                        if camera_model.to_lowercase().starts_with(&camera_make.to_lowercase()) {
                            camera_model.clone()
                        } else {
                            format!("{} {}", &camera_make, camera_model)
                        }
                    })
                } else {
                    camera_model.clone()
                }
            })
    }
}

impl Default for ExifDateDevice {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse an EXIF date string into the date string format for our directories: "YYYY.MM.DD"
fn parse_exif_date(exif_date_str: String, exif_date_format: &str, args: &Args) -> Option<String> {
    let parsed_date_result =
        NaiveDateTime::parse_from_str(exif_date_str.as_str(), exif_date_format);
    match parsed_date_result {
        Ok(date) => {
            let formatted_date = date.format(DATE_DIR_FORMAT).to_string();
            Some(formatted_date)
        }
        Err(err) => {
            if args.debug {
                println!("> could not parse EXIF date {}: {:?}", exif_date_str, err)
            }
            None
        }
    }
}

pub fn read_exif_date_and_device(file: &DirEntry, args: &Args) -> ExifDateDevice {
    let mut exif_data = ExifDateDevice {
        date: None,
        camera_make: None,
        camera_model: None,
    };

    // TODO 5d: handle this unwrap
    // Return early if this is not a file, there's no device name to read
    if file.metadata().unwrap().is_dir() {
        return exif_data;
    }

    // Normally we'd simply call `rexif::parse_file`,
    // but this prints pointless warnings to stderr
    // match rexif::parse_file(&file_name) {
    match read_exif(file.path()) {
        Ok(exif) => {
            // Iterate all EXIF entries and filter only the Model and certain *Date tags
            let _ = &exif.entries.iter().for_each(|exif_entry| {
                match exif_entry.tag {
                    // Camera model
                    ExifTag::Model => {
                        let tag_value = exif_entry.value.to_string().trim().to_string();
                        exif_data.camera_model = Some(tag_value)
                    }

                    // Camera model
                    ExifTag::Make => {
                        let tag_value = exif_entry.value.to_string().trim().to_string();
                        exif_data.camera_make = Some(tag_value)
                    }

                    // Comments based on https://feedback-readonly.photoshop.com/conversations/lightroom-classic/date-time-digitized-and-date-time-differ-from-date-modified-and-date-created/5f5f45ba4b561a3d425c6f77

                    // EXIF:DateTime: When photo software last modified the image or its metadata.
                    // Operating system Date Modified: The time that any application or the camera or
                    // operating system itself modified the file.
                    // Should prefer DateTimeOriginal over this
                    // The String returned by rexif has the standard EXIF format "YYYY:MM:DD HH:MM:SS"
                    ExifTag::DateTime => {
                        let tag_value = exif_entry.value.to_string();
                        if exif_data.date.is_none() {
                            // Only use this if DateTimeOriginal was not found
                            exif_data.date = parse_exif_date(tag_value, REXIF_DATE_FORMAT, args);
                        }
                    }

                    // EXIF:DateTimeOriginal: When the shutter was clicked. Windows File Explorer will display it as Date Taken.
                    // Prefer this over DateTime
                    ExifTag::DateTimeOriginal => {
                        let tag_value = exif_entry.value.to_string();
                        exif_data.date = parse_exif_date(tag_value, REXIF_DATE_FORMAT, args);
                    }

                    // EXIF:DateTimeDigitized: When the image was converted to digital form.
                    // For digital cameras, DateTimeDigitized will be the same as DateTimeOriginal.
                    // For scans of analog pics, DateTimeDigitized is the date of the scan,
                    // while DateTimeOriginal was when the shutter was clicked on the film camera.

                    // We don't need this for now
                    // ExifTag::DateTimeDigitized => {
                    //     ()
                    // }

                    // Ignore other EXIF tags
                    _ => (),
                }
            });
        }

        Err(e) => {
            if args.debug {
                println!("{} could not read EXIF for {:?}: {}",
                         ColoredString::warn_arrow(), file.file_name(), e.to_string());
            }
        }
    }

    exif_data
}

/// Replicate implementation of `rexif::parse_file` and `rexif::read_file`
/// to bypass `rexif::parse_buffer` which prints warnings to stderr
fn read_exif<P: AsRef<Path>>(file_name: P) -> ExifResult {
    // let file_name = file_entry.path();
    // TODO 5d: handle these unwraps
    let mut file = File::open(file_name).unwrap();
    let _ = &file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents: Vec<u8> = Vec::new();
    let _ = &file.read_to_end(&mut contents);
    let (res, _) = rexif::parse_buffer_quiet(&contents);
    res
}

pub fn read_kamadak_exif_date_and_device(file: &DirEntry, args: &Args) -> ExifDateDevice {
    let mut exif_date_device = ExifDateDevice {
        date: None,
        camera_make: None,
        camera_model: None,
    };

    // TODO 5d: handle this unwrap
    // Return early if this is not a file, there's no device name to read
    if file.metadata().unwrap().is_dir() {
        return exif_date_device;
    }

    // Some models are retrieved with extra characters which require removal
    // e.g.: "HUAWEI CAN-L11", ""
    // e.g.: "ALLVIEW P5 camera              "  // <-- yes, lots of extra spaces
    fn clean_device_model_or_make(device_str: &String) -> String {
        device_str
            .replace("\"", "")
            .replace(",", "")
            .trim()
            .to_string()
    }

    match read_kamadak_exif(file.path()) {
        Ok(exif) => {

            exif_date_device.camera_make = exif
                .get_field(Tag::Make, In::PRIMARY)
                .map(|camera_make|{
                    let original_make_str = camera_make.display_value().to_string();
                    let trimmed_make = clean_device_model_or_make(&original_make_str);
                    if args.debug {
                        println!("file '{:?}'", &file.file_name());
                        println!("make: '{}' -> '{}'", original_make_str, &trimmed_make);
                    }
                    clean_device_model_or_make(&trimmed_make)
                });

            if let Some(camera_model) = exif.get_field(Tag::Model, In::PRIMARY) {
                let original_model_str = camera_model.display_value().to_string();
                let trimmed_model = clean_device_model_or_make(&original_model_str);
                if args.debug {
                    println!("model: '{}' -> '{}'", original_model_str, &trimmed_model);
                }
                exif_date_device.camera_model = Some(trimmed_model);
            };

            // EXIF:DateTimeOriginal: When the shutter was clicked. Windows File Explorer will display it as Date Taken.
            // Prefer this over DateTime
            // The display value of the string returned by kamadak-exif has the format "YYYY-MM-DD HH:MM:SS"
            if let Some(date) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
                let tag_value = date.display_value().to_string();
                exif_date_device.date = parse_exif_date(tag_value, KAMADAK_EXIF_DATE_FORMAT, args);

            // EXIF:DateTime: When photo software last modified the image or its metadata.
            // Operating system Date Modified: The time that any application or the camera or
            // operating system itself modified the file.
            // Should prefer DateTimeOriginal over this
            } else if let Some(date) = exif.get_field(Tag::DateTime, In::PRIMARY) {
                let tag_value = date.display_value().to_string();
                exif_date_device.date = parse_exif_date(tag_value, KAMADAK_EXIF_DATE_FORMAT, args);
            };

            // EXIF:DateTimeDigitized: When the image was converted to digital form.
            // For digital cameras, DateTimeDigitized will be the same as DateTimeOriginal.
            // For scans of analog pics, DateTimeDigitized is the date of the scan,
            // while DateTimeOriginal was when the shutter was clicked on the film camera.
            // We don't need DateTimeDigitized for now

            // Ignore other EXIF tags
        }
        Err(e) => {
            if args.debug {
                println!("{} could not read EXIF for {:?}: {}",
                         ColoredString::warn_arrow(), file.file_name(), e.to_string());
            }
        }
    }

    exif_date_device
}

pub fn read_kamadak_exif<P: AsRef<Path>>(file_name: P) -> Result<Exif, Error> {
    let file = std::fs::File::open(file_name)?;
    let mut bufreader = std::io::BufReader::new(&file);
    let exifreader = exif::Reader::new();
    exifreader.read_from_container(&mut bufreader)
}
