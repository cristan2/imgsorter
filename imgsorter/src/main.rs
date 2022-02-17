use std::path::{PathBuf, Path};
use std::{fs, env};
use chrono::{DateTime, Utc};
use std::fs::{DirEntry, DirBuilder, File, Metadata};
use rexif::{ExifEntry, ExifTag, ExifResult};
use std::io::{Read, Seek, SeekFrom};

const DBG_ON: bool = false;
const NO_DATE: &'static str = "no date";

#[derive(Debug)]
pub enum FileType {
    Unknown,
    Image,
    Video,
}

#[derive(Debug)]
pub struct FileStats {
    files_total: i32,
    img_moved: i32,
    img_skipped: i32,
    vid_moved: i32,
    vid_skipped: i32,
    unknown_skipped: i32,
    dirs_skipped: i32,
    dirs_created: i32,
    error_file_move: i32,
    error_dir_creation: i32,
}

impl FileStats {
    pub fn new() -> FileStats {
        FileStats {
            files_total: 0,
            img_moved: 0,
            img_skipped: 0,
            vid_moved: 0,
            vid_skipped: 0,
            unknown_skipped: 0,
            dirs_skipped: 0,
            dirs_created: 0,
            error_file_move: 0,
            error_dir_creation: 0,
        }
    }

    pub fn files_total(&mut self) { self.files_total += 1}
    pub fn img_moved(&mut self) { self.img_moved += 1}
    pub fn img_skipped(&mut self) { self.img_skipped += 1}
    pub fn vid_moved(&mut self) { self.vid_moved += 1}
    pub fn vid_skipped(&mut self) { self.vid_skipped += 1}
    pub fn unknown_skipped(&mut self) { self.unknown_skipped += 1}
    pub fn dirs_skipped(&mut self) { self.dirs_skipped += 1}
    pub fn dirs_created(&mut self) { self.dirs_created += 1}
    pub fn error_file_move(&mut self) { self.error_file_move += 1}
    pub fn error_dir_creation(&mut self) { self.error_dir_creation += 1}
}

fn main() -> Result<(), std::io::Error> {

    let mut stats = FileStats::new();

    // TODO 5: unwrap here?
    let cwd = env::current_dir().unwrap();
    println!("Current working directory is {}", cwd.display());

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
        // TODO 7b: we could skip collecting now, since we'll just iterate the collection later anyway
        .collect::<Vec<DirEntry>>();

    // Iterate files, read modified date and create subdirs
    for dir_entry in dir_contents {

        stats.files_total();

        if DBG_ON {
            /* Print whole entry */
            println!("===============");
            dbg!(&dir_entry);
            println!("---------------");
        }

        let file_name = dir_entry.file_name();
        let dir_entry_metadata: Metadata = dir_entry.metadata()?;

        if dir_entry_metadata.is_dir() {
            println!("Skipping directory {:?}", &file_name);
            stats.dirs_skipped();
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
                    let mut target_subdir = target_dir_path
                        .join(formatted_time_opt.unwrap_or(NO_DATE.to_string()));
                    sort_file_to_subdir(dir_entry, &mut target_subdir, &cwd_path, device_name_opt, file_type, &mut stats)
                },
                FileType::Unknown => {
                    stats.unknown_skipped();
                    println!("Skipping unknown file {:?}", &file_name)
                }
            }
        }
    }

    // Print final stats
    dbg!(&stats);

    Ok(())
}

// TODO 6c: unused, do we need this?
// fn print_file_list(dir_tree: HashMap<String, Vec<DirEntry>>) {
//     dir_tree.iter().for_each(|(dir_name, dir_files)|{
//         println!("{} ({})", dir_name, dir_files.len());
//         for file in dir_files {
//             let filename = &file.file_name();
//             println!("| + {}", filename.to_str().unwrap());
//         }
//         // dbg!(dir_files);
//     })
// }

/// Move the file to a subdirectory named after the file date
/// Optionally, create additional subdir based on device name
fn sort_file_to_subdir(
    file: DirEntry,
    target_subdir: &mut PathBuf,
    cwd_path: &PathBuf,
    device_name_opt: Option<String>,
    file_type: FileType,
    stats: &mut FileStats
) {

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
    create_subdir_if_required(target_subdir, cwd_path, stats);

    // attach file path
    // TODO 5: create new path variable?
    target_subdir.push(&file.file_name());

    // copy file
    // TODO 6a: move instead of copy
    copy_file_if_not_exists(&file, target_subdir, cwd_path, file_type, stats);
}

fn copy_file_if_not_exists(
    file: &DirEntry,
    destination_path: &PathBuf,
    path_cwd: &PathBuf,
    file_type: FileType,
    stats: &mut FileStats
) {
    let file_copy_status = if destination_path.exists() {
        // println!("File already exists, skipping: {:?}", &file.file_name());

        // Record stats for skipped files
        match file_type {
            FileType::Image   => stats.img_skipped(),
            FileType::Video   => stats.vid_skipped(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown => ()
        }
        "already exists"

    } else {
        match fs::copy(file.path(), destination_path) {
            Ok(_) => {
                // Record stats for copied file
                match file_type {
                    FileType::Image   => stats.img_moved(),
                    FileType::Video   => stats.vid_moved(),
                    // don't record any stats for this, shouldn't get one here anyway
                    FileType::Unknown =>()
                }
                "ok"
            },
            Err(err) => {
                println!("File copy error: {:?}: ERROR {:?}", &file.file_name(), err);
                // TODO 5c: log error info
                stats.error_file_move();
                "ERROR"
            }
        }
    };

    // TODO 3a/5c: maybe only log this?
    println!("Copying {} -> {} ... {}",
             // &file.file_name().to_str().unwrap(),
             file.path().strip_prefix(path_cwd).unwrap().display(),
             // &new_file_path.to_str().unwrap()),
             destination_path.strip_prefix(path_cwd).unwrap().display(),
             file_copy_status);
}

// TODO 6b: path_cwd is only required for printlns
fn create_subdir_if_required(target_subdir: &PathBuf, path_cwd: &PathBuf, stats: &mut FileStats) {
    if target_subdir.exists() {
        // TODO 5c: log dir already exists
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
                stats.dirs_created();
                println!("> created subdirectory {}",
                         target_subdir.strip_prefix(&path_cwd).unwrap().display());
            },
            Err(e) => {
                // TODO 2f: handle dir creation fail
                stats.error_dir_creation();
                println!("Failed to create subdirectory {}: {:?}",
                         target_subdir.strip_prefix(&path_cwd).unwrap().display(),
                         e.kind())
            }
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
            // TODO 5c: log this error?
            println!("> can not read EXIF for {:?}: {}", file.file_name(), e.to_string());
            None
        }
    }
}

/// Replicate implementation of `rexif::parse_file` and `rexif::read_file`
/// to bypass `rexif::parse_buffer` which prints warnings to stderr
fn read_exif_file<P: AsRef<Path>>(file_name: P) -> ExifResult {
    // let file_name = file_entry.path();
    let mut file = File::open(file_name).unwrap();
    let _ = &file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents: Vec<u8> = Vec::new();
    let _ = &file.read_to_end(&mut contents);
    let (res, _) = rexif::parse_buffer_quiet(&contents);
    res
}