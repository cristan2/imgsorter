use std::path::{PathBuf, Path};
use std::{fs, env, io};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsString;
use chrono::{DateTime, Utc};
use std::fs::{DirEntry, DirBuilder, File, Metadata};
use rexif::{ExifEntry, ExifTag, ExifResult};
use std::io::{Read, Seek, SeekFrom};

const DBG_ON: bool = false;
const DEFAULT_NO_DATE_STR: &'static str = "no date";
const DEFAULT_TARGET_SUBDIR: &'static str = "imgsorted";
const DEFAULT_MIN_COUNT: i32 = 1;
const DEFAULT_COPY: bool = true;
const DEFAULT_SILENT: bool = false;
const DEFAULT_DRY_RUN: bool = false;

type DeviceTree = HashMap<Option<String>, Vec<SupportedFile>>;
type DateDeviceTree = HashMap<String, DeviceTree>;

#[derive(Debug)]
pub enum FileType {
    Unknown,
    Image,
    Video,
}

pub enum ConfirmationType {
    Proceed,
    DryRun,
    Cancel,
    Error,
}

#[derive(Debug)]
pub struct FileStats {
    files_total: i32,
    img_moved: i32,
    img_copied: i32,
    img_skipped: i32,
    vid_moved: i32,
    vid_copied: i32,
    vid_skipped: i32,
    unknown_skipped: i32,
    dirs_skipped: i32,
    dirs_created: i32,
    error_file_copy: i32,
    error_file_delete: i32,
    error_dir_creation: i32,
}

impl FileStats {
    pub fn new() -> FileStats {
        FileStats {
            files_total: 0,
            img_moved: 0,
            img_copied: 0,
            img_skipped: 0,
            vid_moved: 0,
            vid_copied: 0,
            vid_skipped: 0,
            unknown_skipped: 0,
            dirs_skipped: 0,
            dirs_created: 0,
            error_file_copy: 0,
            error_file_delete: 0,
            error_dir_creation: 0,
        }
    }

    pub fn inc_files_total(&mut self) { self.files_total += 1 }
    pub fn inc_img_moved(&mut self) { self.img_moved += 1 }
    pub fn inc_img_copied(&mut self) { self.img_copied += 1 }
    pub fn inc_img_skipped(&mut self) { self.img_skipped += 1 }
    pub fn inc_vid_moved(&mut self) { self.vid_moved += 1 }
    pub fn inc_vid_copied(&mut self) { self.vid_copied += 1 }
    pub fn inc_vid_skipped(&mut self) { self.vid_skipped += 1 }
    pub fn inc_unknown_skipped(&mut self) { self.unknown_skipped += 1 }
    pub fn inc_dirs_skipped(&mut self) { self.dirs_skipped += 1 }
    pub fn inc_dirs_created(&mut self) { self.dirs_created += 1 }
    pub fn inc_error_file_copy(&mut self) { self.error_file_copy += 1 }
    pub fn inc_error_file_delete(&mut self) { self.error_file_delete += 1 }
    pub fn inc_error_dir_creation(&mut self) { self.error_dir_creation += 1 }
}

#[derive(Debug)]
pub struct SupportedFile {
    file_name: OsString,
    file_path: PathBuf,
    file_type: FileType,
    extension: Option<String>,
    // file's modified date in YYYY-MM-DD format
    date_str: String,
    metadata: Metadata,
    device_name: Option<String>
}

// TODO 5e: find better name
impl SupportedFile {
    pub fn new(dir_entry: DirEntry) -> SupportedFile {
        let _extension = get_extension(&dir_entry);
        let _metadata = dir_entry.metadata().unwrap();
        let _modified_time = get_modified_time(&_metadata);

        SupportedFile {
            file_name: dir_entry.file_name(),
            file_path: dir_entry.path(),
            file_type: get_file_type(&_extension),
            extension: _extension,
            date_str:_modified_time.unwrap_or(DEFAULT_NO_DATE_STR.to_string()),
            metadata: _metadata,
            device_name: get_device_name(&dir_entry)
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
    /// Optionally, a subdir may be set via `set_target_subdir`
    /// where all the sorted files and their date directories
    /// will be created, instead of directly placed in the target_dir
    target_dir: PathBuf,

    /// The minimum number of files with the same date necessary
    /// for a dedicated subdir to be created and the files moved
    min_files_per_dir: i32,

    /// The current working directory
    cwd: PathBuf,

    /// Whether to ask for confirmation before processing files
    silent: bool,

    /// Whether files are copied instead of moved to the sorted subdirs
    copy_not_move: bool,

    /// Whether names of newly created date subdirectories
    /// will include the count of devices and files it contains
    dry_run: bool
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
                cwd,
                silent: DEFAULT_SILENT,
                copy_not_move: DEFAULT_COPY,
                dry_run: DEFAULT_DRY_RUN
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
        silent: Option<bool>,
        copy_not_move: Option<bool>,
        dry_run: Option<bool>
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
                cwd,
                silent: silent.unwrap_or(DEFAULT_SILENT),
                copy_not_move: copy_not_move.unwrap_or(DEFAULT_COPY),
                dry_run: dry_run.unwrap_or(DEFAULT_DRY_RUN),
            }
        )
    }

    /// Change the source directory. This will also change the target
    /// directory to a subdir in the same directory. To set a different
    /// target directory, use [set_target_dir]
    fn set_source_dir(mut self, subdir: &str) -> CliArgs {
        let new_path = PathBuf::from(subdir);
        self.target_dir = new_path.clone().join(DEFAULT_TARGET_SUBDIR);
        self.source_dir = new_path;
        self
    }

    fn set_target_dir(mut self, subdir: &str) -> CliArgs {
        let new_path = PathBuf::from(subdir);
        self.target_dir = new_path.join(DEFAULT_TARGET_SUBDIR);
        self
    }

    fn append_source_subdir(mut self, subdir: &str) -> CliArgs {
        self.source_dir.push(subdir);
        self
    }

    fn append_target_subdir(mut self, subdir: &str) -> CliArgs {
        self.target_dir.push(subdir);
        self
    }

    fn set_silent(mut self, do_proces_silent: bool) -> CliArgs {
        self.silent = do_proces_silent;
        self
    }

    fn set_copy_not_move(mut self, do_copy_not_move_file: bool) -> CliArgs {
        self.copy_not_move = do_copy_not_move_file;
        self
    }

    fn set_dry_run(mut self, do_dry_run: bool) -> CliArgs {
        self.dry_run = do_dry_run;
        self
    }

    fn get_target_dir_ref(&self) -> &PathBuf {
        &self.target_dir
    }

    fn is_dry_run(&self) -> &bool {
        &self.dry_run
    }

    fn is_silent(&self) -> &bool {
        &self.silent
    }
}

fn main() -> Result<(), std::io::Error> {

    let mut stats = FileStats::new();

    let mut args = CliArgs::new()?
        // TODO 1a: temporar citim din ./test_pics
        // .append_source_subdir("test_pics")
        .set_source_dir(r"D:\Temp\New folder test remove")
        .set_silent(false)
        .set_copy_not_move(false);
        // Uncomment for faster dev
        // .set_dry_run(true);

    if DBG_ON {
        dbg!(&args);
    }

    // Read dir contents and filter out error results
    let dir_contents = fs::read_dir(&args.source_dir)?
        .into_iter()
        // filter only ok files
        .filter_map(|entry| entry.ok())

        // filter out any source subdirectories
        // TODO 7c - allow option to recursively walk subdirs
        .filter(|entry| {
            match entry.metadata() {
                Ok(metadata) => {
                    if metadata.is_dir() {
                        if DBG_ON { println!("Skipping directory {:?}", entry.file_name()) }
                        stats.inc_dirs_skipped();
                        false
                    } else {
                        true
                    }
                }
                Err(_) => {
                    println!("Could not read metadata for {:?}", entry);
                    false
                }

            }
        })
        .collect::<Vec<DirEntry>>();

    println!("===========================================================================");
    let copy_status = if args.copy_not_move {"copied:"} else {"moved: "};
    println!("Current working directory is: {}", &args.cwd.display());
    println!("Source directory is:          {}", &args.source_dir.display());
    println!("Target directory is:          {}", &args.target_dir.display());
    println!("Files to be {}           {}", copy_status, dir_contents.len());
    // TODO 1f: print options for this run?
    println!("===========================================================================");

    // Proceed only if user confirms, otherwise exit
    if args.silent {
        println! ("> Silent mode is enabled. Proceeding without user confirmation.");
        if (args.dry_run) {
            println!("> This is a dry run. No folders will be created. No files will be copied or moved.");
        }
    } else {
        match ask_for_confirmation() {
            ConfirmationType::Cancel => {
                println!("Cancelled by user, exiting.");
                return Ok(());
            },
            ConfirmationType::Error => {
                println!("Error confirming, exiting.");
                return Ok(());
            }
            ConfirmationType::DryRun => {
                println!("This is a dry run. No folders will be created. No files will be copied or moved.");
                args.dry_run = true;
                ()
            }
            ConfirmationType::Proceed =>
                ()
        }
    }

    println!("---------------------------------------------------------------------------");
    println!();

    // Iterate files, read modified date and create subdirs
    // Copy images and videos to subdirs based on modified date
    let mut new_dir_tree = parse_dir_contents(dir_contents, &mut stats);

    println!();
    println!("Starting to {} files...", { if args.copy_not_move {"copy"} else {"move"}} );
    println!();

    // Iterate files and either copy/move to subdirs as necessary
    // or do a dry run to simulate a copy/move pass
    process_dir_files(&mut new_dir_tree, &args, &mut stats);

    // Print final stats
    println!();
    dbg!(&stats);

    Ok(())
}

/// Read directory and parse contents into supported data models
/// Return a map of maps to represent the directory tree as below.
/// The map keys are either the date representation or the device name
/// ```
/// [target_dir]          // top-level HashMap
///  └─ [date_dir]        // top-level key of type String
///  │   └─ [device_dir]  // inner HashMap; key of type Option<String>
///  │   │   └─ file.ext  // Vec of supported files
///  │   │   └─ file.ext
///  │   └─ device_dir
///  └─ date_dir
/// ```
fn parse_dir_contents(
    dir_contents: Vec<DirEntry>,
    stats: &mut FileStats
) -> DateDeviceTree {

    let mut new_dir_tree: HashMap<
        String,
        HashMap<
            Option<String>,
            Vec<SupportedFile>>> = HashMap::new();

    for dir_entry in dir_contents {
        stats.inc_files_total();

        let current_file: SupportedFile = SupportedFile::new(dir_entry);

        // Build final target path for this file
        match current_file.file_type {
            FileType::Image | FileType::Video => {
                let file_date = current_file.date_str.clone();
                let file_device = current_file.device_name.clone();

                // Attach file's date as a new subdirectory to the current target path
                let all_devices_for_this_date = new_dir_tree
                    .entry(file_date)
                    .or_insert(HashMap::new());

                let all_files_for_this_device = all_devices_for_this_date
                    .entry(file_device)
                    .or_insert(Vec::new());

                // all_files_for_this_device.push((current_file, destination_path_incl_date));
                all_files_for_this_device.push(current_file);
            },
            FileType::Unknown => {
                stats.inc_unknown_skipped();
                println!("Skipping unknown file {:?}", current_file.get_file_name_ref())
            }
        }
    }

    return new_dir_tree;
}

fn process_dir_files(new_dir_tree: &mut DateDeviceTree, args: &CliArgs, mut stats: &mut FileStats) {
    for (date_dir_name, devices_files_and_paths) in new_dir_tree {
        let device_count_for_date = devices_files_and_paths.keys().len();

        let files_count = devices_files_and_paths.iter()
            .fold(0, |accum, (_, files_and_paths)| accum + files_and_paths.len());

        // let is_dry_run = &args.is_dry_run().clone();
        let is_dry_run = args.dry_run;

        if is_dry_run {
            println!("\n• [{}] ({:?} devices, {:?} files)", date_dir_name, device_count_for_date, files_count)
        }

        for (key_device_name_opt, val_files_and_paths) in devices_files_and_paths {
            let do_create_device_subdirs = device_count_for_date > 1 && key_device_name_opt.is_some();

            // If there's more than one device, create a subdir, otherwise ignore devices
            // if device_count_for_date > 1 && device_name_opt.is_some() {
            if is_dry_run && do_create_device_subdirs {
                println!("   └─ [{}]", key_device_name_opt.clone().unwrap());
            }

            for file in val_files_and_paths {

                // Attach file's date as a new subdirectory to the target path
                let mut destination_path = args.target_dir.clone().join(&date_dir_name);

                if do_create_device_subdirs {
                    // Attach device name as a new subdirectory to the current target path
                    // We could just use the key_device_name_opt, since it's the same value,
                    // but let's just go directly to source just in case
                    let file_device_name_opt = file.get_device_name_ref().clone();

                    // This is safe to unwrap, since we've already checked the device is_some
                    destination_path.push(file_device_name_opt.unwrap());

                    if is_dry_run {
                        println!("   |   └─ {} ===> {}", &file.file_name.to_str().unwrap(), destination_path.display())
                    }
                } else {
                    if is_dry_run {
                        println!("   └─ {} =========> {}", &file.file_name.to_str().unwrap(), destination_path.display())
                    }
                }

                if !is_dry_run {
                    create_subdir_if_required(&destination_path, &args, &mut stats);
                    copy_file_if_not_exists(&file, &mut destination_path, &args, &mut stats);
                }
            }
        } // end loop outer map
    }
}

fn ask_for_confirmation() -> ConfirmationType {
    // TODO 5f: replace '\n'
    println!("OK to proceed? Type one of the options then press Enter:\n• 'y' or 'yes' to continue\n• 'n' or 'no' to cancel\n• 'd' or 'dry' to do a dry run");
    loop {
        let mut user_input = String::new();
        match io::stdin().read_line(&mut user_input) {
            Ok(input) =>
                if DBG_ON {
                    println!("User input: '{:?}'", input)
                },
            Err(err) => {
                    eprintln!("Error reading user input: {:?}", err);
                    return ConfirmationType::Error
                }
        }
        match user_input.trim().to_lowercase().as_str() {
            "n" | "no" =>
                return ConfirmationType::Cancel,
            "y" | "yes" =>
                return ConfirmationType::Proceed,
            "d" | "dry" =>
                return ConfirmationType::DryRun,
            _ =>
                println!("...press one of 'y/yes', 'n/no' or 'd/dry', then Enter")
        }
    }
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

fn copy_file_if_not_exists(
    file: &SupportedFile,
    destination_path: &mut PathBuf,
    args: &CliArgs,
    stats: &mut FileStats
) {

    // attach filename to the directory path
    destination_path.push(file.get_file_name_ref());

    let file_copy_status = if destination_path.exists() {
        if DBG_ON {
            println!("> target file exists: {}",
                     &destination_path.strip_prefix(&args.target_dir).unwrap().display());
        }

        // Record stats for skipped files
        match file.file_type {
            FileType::Image   => stats.inc_img_skipped(),
            FileType::Video   => stats.inc_vid_skipped(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown => ()
        }
        String::from("already exists")

    } else {

        let copy_result = fs::copy(file.get_file_path_ref(), &destination_path);

        match copy_result {
            Ok(_) => {

                // If this is a MOVE, delete the source file after a successful copy
                let delete_result_str = if !args.copy_not_move {

                    let delete_result = fs::remove_file(file.get_file_path_ref());

                    match delete_result {
                        Ok(_) =>
                            String::from(" (source file removed)"),
                        Err(e) => {
                            if DBG_ON { eprintln!("File delete error: {:?}: ERROR {:?}", file.get_file_path_ref(), e) };
                            stats.inc_error_file_delete();
                            String::from(format!(" \x1b[93m(error removing source: {:?})\x1b[0m", e.description()))
                        }
                    }
                } else {
                    String::from("")
                };

                // Record stats for copied file
                match file.file_type {
                    FileType::Image   =>
                        if (args.copy_not_move) { stats.inc_img_copied() } else { stats.inc_img_moved() },
                    FileType::Video   =>
                        if (args.copy_not_move) { stats.inc_vid_copied() } else { stats.inc_vid_moved() },
                    // don't record any stats for this, shouldn't get one here anyway
                    FileType::Unknown =>()
                }
                String::from(format!("OK{}", delete_result_str))
            },
            Err(err) => {
                eprintln!("File copy error: {:?}: ERROR {:?}", file.get_file_path_ref(), err);
                // TODO 5c: log error info
                stats.inc_error_file_copy();
                String::from("ERROR")
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
                println!();
                println!("[Created subdirectory {}]",
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
                "jpg" | "jpeg" | "png" | "tiff" | "crw"| "nef" =>
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
            println!("> could not read EXIF for {:?}: {}", file.file_name(), e.to_string());
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