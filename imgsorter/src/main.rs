use std::path::{PathBuf, Path};
use std::{fs, env};
use std::ffi::OsString;
use chrono::{DateTime, Utc};
use std::fs::{DirEntry, DirBuilder, File, Metadata};
use rexif::{ExifEntry, ExifTag, ExifResult};
use std::io::{Read, Seek, SeekFrom};

const DBG_ON: bool = false;
const NO_DATE_STR: &'static str = "no date";
const DEFAULT_TARGET_SUBDIR: &'static str = "imgsorted";
const DEFAULT_MIN_COUNT: i32 = 1;

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

    pub fn inc_files_total(&mut self) { self.files_total += 1 }
    pub fn inc_img_moved(&mut self) { self.img_moved += 1 }
    pub fn inc_img_skipped(&mut self) { self.img_skipped += 1 }
    pub fn inc_vid_moved(&mut self) { self.vid_moved += 1 }
    pub fn inc_vid_skipped(&mut self) { self.vid_skipped += 1 }
    pub fn inc_unknown_skipped(&mut self) { self.unknown_skipped += 1 }
    pub fn inc_dirs_skipped(&mut self) { self.dirs_skipped += 1 }
    pub fn inc_dirs_created(&mut self) { self.dirs_created += 1 }
    pub fn inc_error_file_move(&mut self) { self.error_file_move += 1 }
    pub fn inc_error_dir_creation(&mut self) { self.error_dir_creation += 1 }
}

#[derive(Debug)]
pub struct SupportedFile {
    file_name: OsString,
    file_path: PathBuf,
    file_type: FileType,
    extension: Option<String>,
    // file modified date in YYYY-MM-DD format
    date_str: String,
    metadata: Metadata,
    device_name: Option<String>
}

// TODO 5e: find better name
impl SupportedFile {
    pub fn new(dir_entry: &DirEntry) -> SupportedFile {
        let _extension = get_extension(dir_entry);
        let _metadata = dir_entry.metadata().unwrap();
        let _modified_time = get_modified_time(&_metadata);

        SupportedFile {
            file_name: dir_entry.file_name(),
            file_path: dir_entry.path(),
            file_type: get_file_type(&_extension),
            extension: _extension,
            date_str:_modified_time.unwrap_or(NO_DATE_STR.to_string()),
            metadata: _metadata,
            device_name: get_device_name(dir_entry)
        }
    }

    pub fn is_dir(&self) -> bool {
        self.metadata.is_dir()
    }

    pub fn get_file_name_ref(&self) -> &OsString {
        &self.file_name
    }

    pub fn get_file_path_ref(&self) -> &PathBuf {
        &self.file_path
    }

    pub fn get_date_str_ref(&self) -> &String {
        &self.date_str
    }

    pub fn get_device_name_ref(&self) -> &Option<String> {
        &self.device_name
    }
}

#[derive(Debug)]
pub struct CliArgs {
    /// The directory where the images to be sorted are located.
    /// If not provided, the current working dir will be used
    source_dir: PathBuf,

    /// The directory where the images to be sorted will be moved.
    /// If not provided, the current working dir will be used.
    /// Additionally, a subdir may be set via `set_target_subdir` where
    /// all the sorted files and their date directories will be created
    target_dir: PathBuf,

    /// The minimum number of files with the same date necessary
    /// for a dedicated subdir to be created and the files moved
    min_files_per_dir: i32,

    /// The current working directory
    cwd: PathBuf
}

impl CliArgs {

    /// Simple constructor using defaults: the CWD is the source
    /// directory and subdir will be created for the target paths
    fn new() -> Result<CliArgs, std::io::Error> {

        let cwd = env::current_dir()?;

        Ok(
            CliArgs {
                source_dir: cwd.clone(),
                target_dir: cwd.clone().join(DEFAULT_TARGET_SUBDIR),
                min_files_per_dir: DEFAULT_MIN_COUNT,
                cwd
            })
    }

    fn new_with_options(
        // Full path from where to read images to be sorted
        source: Option<String>,
        // Subdir inside the CWD from where to read images to be sorted
        // Note: if `source` is provided, this is ignored
        cwd_source_subdir: Option<String>,
        // Full path where the sorted images will be moved
        target: Option<String>,
        // Subdir inside the CWD where the sorted images will be moved
        // Note: if `target` is provided, this is ignored
        cwd_target_subdir: Option<String>,
        min_files: Option<i32>,
    ) -> Result<CliArgs, std::io::Error> {

        fn create_path(provided_path: Option<String>, path_subdir: Option<String>, cwd: &PathBuf) -> PathBuf {
            match provided_path {
                // if a full path has been provided, use that
                Some(path) =>
                    PathBuf::from(path),
                // otherwise, use the cwd...
                None => {
                    // but create a subdir if one was provided
                    match path_subdir {
                        Some(subdir) =>
                            cwd.join(subdir),
                        None =>
                            cwd.clone()
                    }
                }
            }
        }

        let cwd = env::current_dir()?;

        Ok(
            CliArgs {
                source_dir: create_path(source, cwd_source_subdir, &cwd),
                target_dir: create_path(
                    target,
                    cwd_target_subdir.or(Some(String::from(DEFAULT_TARGET_SUBDIR))),
                    &cwd),
                min_files_per_dir: min_files.unwrap_or(DEFAULT_MIN_COUNT),
                cwd
            }
        )
    }

    fn set_source_subdir(mut self, subdir: &str) -> CliArgs {
        self.source_dir.push(subdir);
        self
    }

    fn set_target_subdir(mut self, subdir: &str) -> CliArgs {
        self.target_dir.push(subdir);
        self
    }
}

fn main() -> Result<(), std::io::Error> {

    let mut stats = FileStats::new();

    let args = CliArgs::new()?
        // TODO 1a: temporar citim din ./test_pics
        .set_source_subdir("test_pics");

    if DBG_ON {
        dbg!(&args);
    }

    println!("====================================================================");
    println!("Current working directory is {}", &args.cwd.display());
    println!("Source directory is {}", &args.source_dir.display());
    println!("Target directory is {}", &args.target_dir.display());
    println!("====================================================================");

    // Read dir contents and filter out error results
    let dir_contents = fs::read_dir(&args.source_dir)?
        .into_iter()
        .filter_map(|entry| entry.ok())
        // TODO 7b: we could skip collecting now, since we'll just iterate the collection later anyway
        .collect::<Vec<DirEntry>>();

    // Iterate files, read modified date and create subdirs
    for dir_entry in dir_contents {

        stats.inc_files_total();

        let current_file: &SupportedFile = &SupportedFile::new(&dir_entry);

        if DBG_ON {
            /* Print whole entry */
            println!("===============");
            dbg!(&dir_entry);
            println!("---------------");
            dbg!(current_file);
            println!("---------------");
        }

        if current_file.is_dir() {
            println!("Skipping directory {:?}",current_file.file_name);
            stats.inc_dirs_skipped();
        } else {

            // Copy images and videos to subdirs based on modified date
            // TODO 6a: move instead of copy
            match current_file.file_type {
                FileType::Image | FileType::Video => {
                    // Attach file's date as a new subdirectory to the current target path
                    let target_subdir = &args.target_dir.join(current_file.get_date_str_ref());
                    sort_file_to_subdir(current_file, target_subdir, &args, &mut stats)
                },
                FileType::Unknown => {
                    stats.inc_unknown_skipped();
                    println!("Skipping unknown file {:?}", current_file.get_file_name_ref())
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
    file: &SupportedFile,
    date_subdir: &PathBuf,
    args: &CliArgs,
    stats: &mut FileStats
) {

    // Attach device name as a new subdirectory to the current target path
    let mut target_subdir: PathBuf = match file.get_device_name_ref() {
        Some(device_name) =>
            // TODO 4a - replace device name with custom name from config
            date_subdir.join(&device_name),
        None =>
            date_subdir.clone()
    };

    if DBG_ON {
        println!("Source dir = {:?}", args.source_dir);
        println!("Date dir = {:?}", date_subdir);
        println!("Target subdir = {:?}", target_subdir);
    }

    // create target subdir
    create_subdir_if_required(&target_subdir, args, stats);

    // attach filename to the directory path
    // TODO 5: create new path variable?
    target_subdir.push(file.get_file_name_ref());

    // copy file
    // TODO 6a: move instead of copy
    copy_file_if_not_exists(file, &target_subdir, args, stats);
}

fn copy_file_if_not_exists(
    file: &SupportedFile,
    destination_path: &PathBuf,
    args: &CliArgs,
    stats: &mut FileStats
) {
    let file_copy_status = if destination_path.exists() {
        // println!("File already exists, skipping: {:?}", &file.file_name());

        // Record stats for skipped files
        match file.file_type {
            FileType::Image   => stats.inc_img_skipped(),
            FileType::Video   => stats.inc_vid_skipped(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown => ()
        }
        "already exists"

    } else {
        let copy_result = fs::copy(file.get_file_path_ref(), destination_path);
        match copy_result {
            Ok(_) => {
                // Record stats for copied file
                match file.file_type {
                    FileType::Image   => stats.inc_img_moved(),
                    FileType::Video   => stats.inc_vid_moved(),
                    // don't record any stats for this, shouldn't get one here anyway
                    FileType::Unknown =>()
                }
                "OK"
            },
            Err(err) => {
                println!("File copy error: {:?}: ERROR {:?}", file.get_file_path_ref(), err);
                // TODO 5c: log error info
                stats.inc_error_file_move();
                "ERROR"
            }
        }
    };

    // TODO 3a/5c: maybe only log this?
    println!("Copying {} -> {} ... {}",
             // &file.file_name().to_str().unwrap(),
             file.get_file_path_ref().strip_prefix(&args.source_dir).unwrap().display(),
             // &new_file_path.to_str().unwrap()),
             destination_path.strip_prefix(&args.target_dir).unwrap().display(),
             file_copy_status);
}

fn create_subdir_if_required(
    target_subdir: &PathBuf,
    args: &CliArgs,
    stats: &mut FileStats
) {
    if target_subdir.exists() {
        if DBG_ON {
            println!("> target subdir exists: {}",
                     &target_subdir.strip_prefix(&args.target_dir).unwrap().display());
        }
    } else {
        let subdir_creation = DirBuilder::new()
            // create subdirs if necessary; don't return Err if file exists
            .recursive(true)
            .create(target_subdir);

        match subdir_creation {
            Ok(_) => {
                stats.inc_dirs_created();
                println!("> created subdirectory {}",
                         target_subdir.strip_prefix(&args.target_dir).unwrap().display());
            },
            Err(e) => {
                // TODO 2f: handle dir creation fail
                stats.inc_error_dir_creation();
                println!("Failed to create subdirectory {}: {:?}",
                         target_subdir.strip_prefix(&args.target_dir).unwrap().display(),
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

    // TODO 5d: handle this unwrap
    // Return early if this is not a file, there's no device name to read
    if file.metadata().unwrap().is_dir() {
        return None
    }

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
    // TODO 5d: handle these unwraps
    let mut file = File::open(file_name).unwrap();
    let _ = &file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents: Vec<u8> = Vec::new();
    let _ = &file.read_to_end(&mut contents);
    let (res, _) = rexif::parse_buffer_quiet(&contents);
    res
}