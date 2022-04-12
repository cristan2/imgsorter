use std::path::Path;
use std::fs::{DirEntry, File};
use std::io::{Read, Seek, SeekFrom};

use rexif::{ExifTag, ExifResult};
use chrono::NaiveDateTime;

use crate::config::*;
use crate::utils::*;

/// Selected EXIF Data for a [[SupportedFile]]
/// Currently includes only the image date and camera model
#[derive(Debug)]
pub struct ExifDateDevice {
    pub date_time: Option<String>,
    pub date_original: Option<String>,
    pub camera_model: Option<String>
}

impl ExifDateDevice {
    pub fn new() -> ExifDateDevice {
        ExifDateDevice {
            date_original: None,
            date_time: None,
            camera_model: None
        }
    }
}

/// Read a String in standard EXIF format "YYYY:MM:DD HH:MM:SS"
/// and try to parse it into the date format for our directories: "YYYY.MM.DD"
fn parse_exif_date(date_str: String, args: &Args) -> Option<String> {
    let parsed_date_result = NaiveDateTime::parse_from_str(date_str.as_str(), "%Y:%m:%d %H:%M:%S");
    match parsed_date_result {
        Ok(date) => {
            let formatted_date = date.format(DATE_DIR_FORMAT).to_string();
            Some(formatted_date)
        }
        Err(err) => {
            if args.debug { println!("> could not parse EXIF date {}: {:?}", date_str, err) }
            None
        }
    }
}

pub fn read_exif_date_and_device(
    file: &DirEntry,
    args: &Args
) -> ExifDateDevice {

    let mut exif_data = ExifDateDevice {
        date_original: None,
        date_time: None,
        camera_model: None
    };

    // TODO 5d: handle this unwrap
    // Return early if this is not a file, there's no device name to read
    if file.metadata().unwrap().is_dir() {
        return exif_data
    }

    // Normally we'd simply call `rexif::parse_file`,
    // but this prints pointless warnings to stderr
    // match rexif::parse_file(&file_name) {
    match read_exif(file.path()) {

        Ok(exif) => {
            // Iterate all EXIF entries and filter only the Model and certain *Date tags
            let _ = &exif.entries.iter()
                .for_each(|exif_entry| {
                    match exif_entry.tag {

                        // Camera model
                        ExifTag::Model => {
                            let tag_value = exif_entry.value.to_string().trim().to_string();
                            exif_data.camera_model = Some(tag_value)
                        },

                        // Comments based on https://feedback-readonly.photoshop.com/conversations/lightroom-classic/date-time-digitized-and-date-time-differ-from-date-modified-and-date-created/5f5f45ba4b561a3d425c6f77

                        // EXIF:DateTime: When photo software last modified the image or its metadata.
                        // Operating system Date Modified: The time that any application or the camera or
                        // operating system itself modified the file.
                        // The String returned by rexif has the standard EXIF format "YYYY:MM:DD HH:MM:SS"
                        ExifTag::DateTime => {
                            let tag_value = exif_entry.value.to_string();
                            exif_data.date_time = parse_exif_date(tag_value, args);
                        }

                        // EXIF:DateTimeOriginal: When the shutter was clicked. Windows File Explorer will display it as Date Taken.
                        ExifTag::DateTimeOriginal => {
                            let tag_value = exif_entry.value.to_string();
                            exif_data.date_original = parse_exif_date(tag_value, args);
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
                        _ =>
                            ()
                    }
                });
        },

        Err(e) => {
            // TODO 5c: log this error?
            if args.verbose {
                println!("{} could not read EXIF for {:?}: {}", ColoredString::warn_arrow(), file.file_name(), e.to_string());
            }
        }
    }

    return exif_data;
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