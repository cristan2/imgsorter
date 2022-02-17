use std::path::{PathBuf, Path};
use std::{fs, env};
use chrono::{DateTime, Utc};
use std::fs::{DirEntry, DirBuilder, File, Metadata};
use std::collections::HashMap;
use rexif::{ExifEntry, ExifTag, ExifResult};
use std::io::{Read, Seek, SeekFrom};

const DBG_ON: bool = true;
const NO_DATE: &'static str = "no date";

#[derive(Debug)]
pub enum FileType {
    Unknown,
    Image,
    Video,
}

fn main() -> Result<(), std::io::Error> {

    // TODO 5: unwrap here?
    let cwd = env::current_dir().unwrap();
    println!("current working directory = {}", cwd.display());

    // TODO 1a: temporary citim din ./test_pics, normal ar trebui sa fie argument sau doar current dir
    // let path_cwd = Path::new(".")
    let cwd_path: PathBuf = cwd.join("test_pics");

    // Create target subdir based on image date
    // TODO 1d: based on target flag to create subdir for sorted files, or sort in-place?
    let target_dir_path = cwd_path.join("imgsorted");

    // Read dir contents and filter out error results
    let dir_contents = fs::read_dir(&cwd_path)?
        .into_iter()
        .filter_map(|entry| entry.ok())
        .collect::<Vec<DirEntry>>();

    // TODO 3a: add printout no of files
    // iterate files, read modified date and create subdirs
    for dir_entry in dir_contents {

        if DBG_ON {
            /* Print whole entry */
            println!("===============");
            dbg!(&dir_entry);
            println!("---------------");
        }

        let file_name = dir_entry.file_name();
        let dir_entry_metadata: Metadata = dir_entry.metadata()?;

        if dir_entry_metadata.is_dir() {
            println!("{:?} is a directory, ignoring", &file_name);
        } else {

            let extension_opt: Option<String> = get_extension(&dir_entry);
            let file_type = get_file_type(&extension_opt);
            let formatted_time_opt: Option<String> = get_modified_time(&dir_entry_metadata);
            let device_name_opt: Option<String> = get_device_name(&dir_entry);

            // TODO 5a: parse file to "supported file" struct

            if DBG_ON {
                if let Some(ext) = extension_opt {
                    println!("File {:?} has extension '{}'", &file_name, ext);
                    println!("File type for extension {:?} is {:?}", &ext, &file_type);
                } else {
                    println!("No extension for : '{}'", &dir_entry.path().to_str().unwrap_or("??"));
                }
            }

            // Copy images and videos to subdirs based on modified date
            // TODO 6a: move instead of copy
            match file_type {
                FileType::Image | FileType::Video => {

                    let mut target_subdir = match formatted_time_opt {
                        Some(date) =>
                            target_dir_path.join(date),
                        None =>
                            target_dir_path.join(NO_DATE.to_string())
                    };

                    sort_file_to_subdir(dir_entry, &mut target_subdir, &cwd_path, device_name_opt)
                },
                FileType::Unknown =>
                    println!("Skipping unknown file {:?}", &file_name)
            }
        }
    }

    Ok(())
}

fn print_file_list(dir_tree: HashMap<String, Vec<DirEntry>>) {
    dir_tree.iter().for_each(|(dir_name, dir_files)|{
        println!("{} ({})", dir_name, dir_files.len());
        for file in dir_files {
            let filename = &file.file_name();
            println!("| + {}", filename.to_str().unwrap());
        }
        // dbg!(dir_files);
    })
}

/// Move the file to a subdirectory named after the file date
/// Optionally, create additional subdir based on device name
fn sort_file_to_subdir(file: DirEntry, target_subdir: &mut PathBuf, cwd_path: &PathBuf, device_name_opt: Option<String>) {

    // attach device name subdir path
    if let Some(device_name) = device_name_opt {
        // TODO 4a - replace device name with custom name from config
        target_subdir.push(&device_name);
    }

    if DBG_ON {
        println!("Current dir = {:?}", cwd_path);
        println!("Target subdir = {:?}", target_subdir);
    }

    // create target subdir
    create_subdir_if_required(target_subdir, cwd_path);

    // attach file path
    // TODO 5: create new path variable?
    target_subdir.push(&file.file_name());

    // copy file
    // TODO 6a: move instead of copy
    copy_file_if_not_exists(&file, target_subdir, cwd_path);
}

fn copy_file_if_not_exists(file: &DirEntry, target_subdir: &PathBuf, path_cwd: &PathBuf) {
    let file_copy_status = if target_subdir.exists() {
        "already exists"
    } else {
        match fs::copy(file.path(), target_subdir) {
            Ok(_) =>
                "ok",
            Err(err) =>
                // println!("File copy error: {:?}: ERROR {:?}", &file.file_name(), err)
                // TODO add error info
                "ERROR"
        }
    };

    println!("Copying {} -> {} ... {}",
             // &file.file_name().to_str().unwrap(),
             file.path().strip_prefix(path_cwd).unwrap().display(),
             // &new_file_path.to_str().unwrap()),
             target_subdir.strip_prefix(path_cwd).unwrap().display(),
             file_copy_status);
}

// TODO 6b: path_cwd is only required for printlns
fn create_subdir_if_required(target_subdir: &PathBuf, path_cwd: &PathBuf) {
    if target_subdir.exists() {
        // TODO 2f: handle dir already exists, maybe just log it?
        // println!("target dir exists: {}: {}",
        //          &target_subdir.strip_prefix(&path_cwd).unwrap().display(),
        //          &target_subdir.exists());
        // "already exists"
    } else {
        let subdir_creation = DirBuilder::new()
            // create subdirs if necessary; don't return Err if file exists
            .recursive(true)
            .create(target_subdir);

        match subdir_creation {
            Ok(_) => {
                println!("(Created subdirectory {})",
                         target_subdir.strip_prefix(&path_cwd).unwrap().display());
            },
            Err(e) =>
                // TODO 2f: handle dir creation fail
                println!("Failed to create subdirectory {}: {:?}",
                         target_subdir.strip_prefix(&path_cwd).unwrap().display(),
                         e.kind())
        }
    };
}

/// Read metadata and return file modified time in YYYY-MM-DD format
fn get_modified_time(file_metadata: &Metadata) -> Option<String> {
    file_metadata.modified().map_or(None, |system_time| {
        let datetime: DateTime<Utc> = system_time.into(); // 2021-06-05T16:26:22.756168300Z
        Some(datetime.format("%Y.%m.%d").to_string())
    })
}

fn get_extension(file: &DirEntry) -> Option<String> {
    file.path()
        .extension()
        .map_or(None, |os| {
            os.to_str().map_or(None, |s|
                Some(String::from(s)))
        })
}

/// Determine the type of file based on the file extension
/// Return one of Image|Video|Unknown enum types
fn get_file_type(extension_opt: &Option<String>) -> FileType {
    match extension_opt {
        Some(extension) => {
            match extension.to_lowercase().as_str() {
                "jpg" | "jpeg" | "png" | "tiff" =>
                    FileType::Image,
                "mp4" | "mov" | "3gp" =>
                    FileType::Video,
                _ =>
                    FileType::Unknown
            }
        }
        None =>
            FileType::Unknown,
    }
}

fn get_device_name(file: &DirEntry) -> Option<String> {
    let file_name = file.path();

    // Normally we'd simply call `rexif::parse_file`,
    // but this prints pointless warnings to stderr
    // match rexif::parse_file(&file_name) {
    match read_exif_file(&file_name) {

        Ok(exif) => {
            let model = &exif.entries.iter()
                .filter(|exif_entry| {
                    exif_entry.tag == ExifTag::Model})
                .map(|e| e)
                .collect::<Vec<&ExifEntry>>();

            return match model.len() {
                0 => None,
                _len => {
                    let model_name = model.get(0).map_or(None, |entry| {
                        let s = entry.value.to_string().trim().to_string();
                        Some(s)
                    });
                    model_name
                }
            }
        },

        Err(e) => {
            // dbg!(e);
            println!("Can not read EXIF for {:?}: {}", &file_name, e.to_string());
            None
        }
    }
}

/// Replicate implementation of `rexif::parse_file` and `rexif::read_file`
/// to bypass `rexif::parse_buffer` which prints warnings to stderr
fn read_exif_file<P: AsRef<Path>>(file_name: P) -> ExifResult {
    // let file_name = file_entry.path();
    let mut file = File::open(file_name).unwrap();
    &file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents: Vec<u8> = Vec::new();
    &file.read_to_end(&mut contents);
    let (res, _) = rexif::parse_buffer_quiet(&contents);
    res
}