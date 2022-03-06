use std::path::{PathBuf, Path};
use std::{fs, env, io};
use std::cmp::max;
use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use chrono::{DateTime, NaiveDateTime, Utc};
use std::fs::{DirEntry, DirBuilder, File, Metadata};
use rexif::{ExifTag, ExifResult};
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use imgsorter::utils::*;

const DBG_ON: bool = false;
const DEFAULT_NO_DATE_STR: &'static str = "no date";
const DEFAULT_TARGET_SUBDIR: &'static str = "imgsorted";
const DEFAULT_MIN_COUNT: i32 = 1;
const DEFAULT_COPY: bool = true;
const DEFAULT_SILENT: bool = false;
const DEFAULT_DRY_RUN: bool = false;

const DATE_DIR_FORMAT: &'static str = "%Y.%m.%d";

/// Convenience alias over a map meant to represent
/// a string representation of a device holding a Vec of files
struct DeviceTree {
    file_tree: BTreeMap<Option<String>, Vec<SupportedFile>>,
    max_dir_path_len: usize
}

impl DeviceTree {
    fn new() -> DeviceTree {
        DeviceTree {
            file_tree: BTreeMap::new(),
            max_dir_path_len: 0,
        }
    }
}

/// A wrapper over a map of maps to represent the directory tree as described below.
/// The outer map keys is the date representation
/// The inner map keys is an Optional device name
/// Use BTreeMap's to have the keys sorted
/// ```
/// [target_dir]          // top-level map
///  └─ [date_dir]        // top-level key of type String
///  │   └─ [device_dir]  // inner map; key of type Option<String>
///  │   │   └─ file.ext  // inner map; value is Vec of supported files
///  │   │   └─ file.ext
///  │   └─ device_dir
///  └─ date_dir
/// ```
/// Additionally, the struct
struct DateDeviceTree {
    dir_tree: BTreeMap<String, DeviceTree>,
    max_filename_len: usize,
    max_path_len: usize
}

impl DateDeviceTree {
    fn new() -> DateDeviceTree {
        DateDeviceTree {
            dir_tree: BTreeMap::new(),
            max_filename_len: 0,
            max_path_len: 0,
        }
    }

    // Find the maximum length of the path string that may be present in the output
    // This can only be computed after the tree has been filled with devices and files
    // because of the requirement to only create device subdirs if there are at least 2 devices
    // Resulting value covers two cases:
    // - there's at least one date dir with >1 devices subdirs: compute path length to include `date/device_name/file_name`
    // - there's no date dir with >1 devices: compute path length to include `date/file_name`
    fn compute_max_path_len(&mut self) {
        let max_date_dir_path_len = &self.dir_tree.iter()
            // filter only date dirs with at least 2 devices
            .filter(|(_, device_tree)| device_tree.file_tree.keys().clone().len() > 1 )
            // now search all devices for the max path len
            .map(|(_, device_tree)| device_tree.max_dir_path_len)
            .max();

        if max_date_dir_path_len.is_some() {
            // add +1 for the length of the separator between dirs and filename
            self.max_path_len = max_date_dir_path_len.clone().unwrap() + 1 + &self.max_filename_len;
        } else {
            // add +10 for the length of date dirs, e.g. 2016.12.29
            // add +1 for the length of the separator between date and filename
            self.max_path_len = 10 + 1 + &self.max_filename_len;
        }
    }
}

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
    dirs_ignored: i32,
    dirs_created: i32,
    error_file_create: i32,
    error_file_delete: i32,
    error_dir_create: i32,
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
            dirs_ignored: 0,
            dirs_created: 0,
            error_file_create: 0,
            error_file_delete: 0,
            error_dir_create: 0,
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
    pub fn inc_dirs_ignored(&mut self) { self.dirs_ignored += 1 }
    pub fn inc_dirs_created(&mut self) { self.dirs_created += 1 }
    pub fn inc_error_file_create(&mut self) { self.error_file_create += 1 }
    pub fn inc_error_file_delete(&mut self) { self.error_file_delete += 1 }
    pub fn inc_error_dir_create(&mut self) { self.error_dir_create += 1 }

    pub fn color_if_non_zero(err_stat: i32, level: OutputColor) -> String {
        if err_stat > 0 {
            match level {
                OutputColor::Error =>
                    ColoredString::red(err_stat.to_string().as_str()),
                OutputColor::Warning =>
                    ColoredString::orange(err_stat.to_string().as_str()),
                OutputColor::Neutral =>
                    ColoredString::bold_white(err_stat.to_string().as_str()),
                OutputColor::Good =>
                    ColoredString::green(err_stat.to_string().as_str()),
            }
        } else {
            String::from(err_stat.to_string())
        }
    }

    pub fn print_stats(&self, args: &CliArgs) {
        let general_stats = format!("
Final statistics
-----------------------------
Total files:             {total}
-----------------------------
Images moved:            {img_move}
Images copied:           {img_copy}
Images skipped:          {img_skip}
Videos moved:            {vid_move}
Videos copied:           {vid_copy}
Videos skipped:          {vid_skip}
Directories ignored:     {dir_ignore}
Directories created:     {dir_create}
Unknown files skipped:   {f_skip}
-----------------------------
File create errors:      {fc_err}
File delete errors:      {fd_err}
Directory create errors: {dc_err}
-----------------------------",
                            total=FileStats::color_if_non_zero(self.files_total, OutputColor::Neutral),
                            img_move=FileStats::color_if_non_zero(self.img_moved, OutputColor::Neutral),
                            img_copy=FileStats::color_if_non_zero(self.img_copied, OutputColor::Neutral),
                            img_skip=FileStats::color_if_non_zero(self.img_skipped, OutputColor::Warning),
                            vid_move=FileStats::color_if_non_zero(self.vid_moved,OutputColor::Neutral),
                            vid_copy=FileStats::color_if_non_zero(self.vid_copied,OutputColor::Neutral),
                            vid_skip=FileStats::color_if_non_zero(self.vid_skipped, OutputColor::Warning),
                            dir_create=FileStats::color_if_non_zero(self.dirs_created, OutputColor::Neutral),
                            dir_ignore=FileStats::color_if_non_zero(self.dirs_ignored, OutputColor::Warning),
                            f_skip=FileStats::color_if_non_zero(self.unknown_skipped, OutputColor::Warning),
                            fc_err=FileStats::color_if_non_zero(self.error_file_create, OutputColor::Error),
                            fd_err=FileStats::color_if_non_zero(self.error_file_delete, OutputColor::Error),
                            dc_err=FileStats::color_if_non_zero(self.error_dir_create, OutputColor::Error),
        );

        println!("{}", general_stats);

        if self.files_total == self.unknown_skipped {
            println!("{}", ColoredString::orange("No supported files found in source folder."))
        } else {
            if self.error_file_create > 0 {
                println!("{} Some files could not be created in the target path", ColoredString::warn_arrow())
            }
    
            if !args.copy_not_move && self.error_file_delete > 0  {
                println!("{} Some files were copied but the source files could not be removed", ColoredString::warn_arrow())
            }
        }
    }
}

/// Selected EXIF Data for a [[SupportedFile]]
/// Currently includes only the image date and camera model
#[derive(Debug)]
pub struct ExifDateDevice {
    date_time: Option<String>,
    date_original: Option<String>,
    camera_model: Option<String>
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
    device_name: Option<String>,
}

// TODO 5e: find better name
impl SupportedFile {
    pub fn parse_from(dir_entry: DirEntry) -> SupportedFile {
        let _extension = get_extension(&dir_entry);
        let _metadata = dir_entry.metadata().unwrap();

        let _exif_data = read_exif_date_and_device(&dir_entry);
        let _system_date = get_system_modified_date(&_metadata);

        // Read image date - prefer EXIF tags over system date
        let _image_date = _exif_data.date_original
            .unwrap_or(_exif_data.date_time
                .unwrap_or(_system_date
                    .unwrap_or(DEFAULT_NO_DATE_STR.to_string())));

        SupportedFile {
            file_name: dir_entry.file_name(),
            file_path: dir_entry.path(),
            file_type: get_file_type(&_extension),
            extension: _extension,
            date_str: _image_date,
            metadata: _metadata,
            device_name: _exif_data.camera_model
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
}

fn main() -> Result<(), std::io::Error> {

    let mut stats = FileStats::new();

    let mut args = CliArgs::new()?
        // TODO 1a: temporar citim din ./test_pics
        // .append_source_subdir("test_pics")
        .set_source_dir(r"D:\Temp\New folder test remove - Copy")
        .set_silent(false)
        .set_copy_not_move(true);
        // Uncomment for faster dev
        // .set_dry_run(true);

    if DBG_ON {
        dbg!(&args);
    }

    // TODO 6f: handle path not exists
    // Read dir contents and filter out error results
    let dir_contents = read_supported_files(&mut stats, &mut args)?;

    let copy_status = if args.copy_not_move {
        ColoredString::orange("copied:")
    } else {
        ColoredString::red("moved: ")
    };
    // TODO 1f: print options for this run?
    println!("===========================================================================");
    println!("Current working directory is: {}", &args.cwd.display());
    println!("Source directory is:          {}", &args.source_dir.display());
    println!("Target directory is:          {}", &args.target_dir.display());
    println!("Files to be {}           {}", copy_status, dir_contents.len());
    println!("===========================================================================");

    // Proceed only if silent is enabled or user confirms, otherwise exit
    if args.silent {
        println! ("> Silent mode is enabled. Proceeding without user confirmation.");
        if args.dry_run {
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

    let start_time = Instant::now();

    println!("---------------------------------------------------------------------------");
    println!();

    // Iterate files, read modified date and create subdirs
    // Copy images and videos to subdirs based on modified date
    let mut new_dir_tree = parse_dir_contents(dir_contents, &mut stats);

    if !new_dir_tree.dir_tree.is_empty() {
        println!();
        let start_status = format!("Starting to {} files...", { if args.copy_not_move {"copy"} else {"move"}} );
        println!("{}", ColoredString::bold_white(start_status.as_str()));
        println!();
    
        // Iterate files and either copy/move to subdirs as necessary
        // or do a dry run to simulate a copy/move pass
        process_dir_files(&mut new_dir_tree, &args, &mut stats);
    }

    // Print final stats
    println!();
    stats.print_stats(&args);

    let duration = start_time.elapsed();
    println!("Finished in {}.{} sec", duration.as_secs(), duration.subsec_millis());

    Ok(())
}

/// Read contents of source dir and filter out directories or those which failed to read
fn read_supported_files(stats: &mut FileStats, args: &mut CliArgs) -> Result<Vec<DirEntry>, std::io::Error> {
    Ok(
        fs::read_dir(&args.source_dir)?
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
                            stats.inc_dirs_ignored();
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

            .collect::<Vec<DirEntry>>())
}

/// Read directory and parse contents into supported data models
fn parse_dir_contents(
    dir_contents: Vec<DirEntry>,
    stats: &mut FileStats
) -> DateDeviceTree {

    let mut new_dir_tree: DateDeviceTree = DateDeviceTree::new();

    for dir_entry in dir_contents {
        stats.inc_files_total();

        let current_file: SupportedFile = SupportedFile::parse_from(dir_entry);

        // Build final target path for this file
        match current_file.file_type {
            FileType::Image | FileType::Video => {
                let file_date = current_file.date_str.clone();
                let file_device = current_file.device_name.clone();

                // Attach file's date as a new subdirectory to the current target path
                let all_devices_for_this_date = new_dir_tree.dir_tree
                    .entry(file_date)
                    .or_insert(DeviceTree::new());

                let all_files_for_this_device = all_devices_for_this_date.file_tree
                    .entry(file_device)
                    .or_insert(Vec::new());

                // Store the string lengths of the file name and path for padding in stdout
                let _filename_len = String::from(current_file.file_name.clone().to_str().unwrap()).chars().count();
                let _device_name_len = current_file.device_name.clone().map(|d|d.chars().count()).unwrap_or(0);
                let _date_name_str = &current_file.date_str.chars().count();
                // add +1 for each path separator character
                let total_target_path_len = _date_name_str + 1 + _device_name_len;

                new_dir_tree.max_filename_len = max(new_dir_tree.max_filename_len, _filename_len);
                all_devices_for_this_date.max_dir_path_len = max(all_devices_for_this_date.max_dir_path_len, total_target_path_len);

                // Add file to dir tree
                all_files_for_this_device.push(current_file);
            },

            FileType::Unknown => {
                stats.inc_unknown_skipped();
                println!("Skipping unknown file {:?}", current_file.get_file_name_ref())
            }
        }
    }

    // The max path length can only be computed after the tree has been filled with devices and files
    // because of the requirement to only create device subdirs if there are at least 2 devices
    new_dir_tree.compute_max_path_len();

    return new_dir_tree;
}

fn process_dir_files(new_dir_tree: &mut DateDeviceTree, args: &CliArgs, mut stats: &mut FileStats) {

    let is_dry_run = args.dry_run;

    // Dry runs will output a dirtree-like structure, so add the additional
    // indents and markings to the max length to be taken into account when padding
    if is_dry_run {
        new_dir_tree.max_filename_len = new_dir_tree.max_filename_len
            + String::from(FILE_TREE_INDENT).chars().count()
            + String::from(FILE_TREE_ENTRY).chars().count()
    }

    let dir_padding_width = {
        if is_dry_run {
            let _total_padding_width = {
                new_dir_tree.max_filename_len
                    + 1 // add +1 for the gap between a filename and its padding
                    + SEPARATOR_DRY_RUN.chars().count()
                    + new_dir_tree.max_path_len
                    + SEPARATOR_STATUS.chars().count()
                    + 1 // add +1 for the gap between a path and its padding
                    + 1 // add +1 for the gap between a path and the operation status
            };
            Some(_total_padding_width)
        } else { None }
    };

    /*****************************************************************************/
    /* ---             Iterate each date directory to be created             --- */
    /*****************************************************************************/

    for (date_dir_name, devices_files_and_paths) in &new_dir_tree.dir_tree {
        let device_count_for_date = devices_files_and_paths.file_tree.keys().len();

        let files_count = devices_files_and_paths.file_tree.iter()
            .fold(0, |accum, (_, files_and_paths)|
                accum + files_and_paths.len());

        // Attach file's date as a new subdirectory to the target path
        let date_destination_path = args.target_dir.clone().join(date_dir_name);

        if is_dry_run {

            let _dir_name_with_device_status = format!("[{}] ({:?} devices, {:?} files) ",
                                                        date_dir_name.clone(),
                                                        device_count_for_date,
                                                        files_count);

            let padded_dir_name = RightPadding::dot(
                _dir_name_with_device_status,
                // safe to unwrap for dry runs
                dir_padding_width.unwrap());

            // Check restrictions - if target exists
            let target_dir_exists = dry_run_check_target_exists(&date_destination_path);

            // Print everything together
            println!("\n{} {}", padded_dir_name, target_dir_exists);
        }


        /*****************************************************************************/
        /* ---            Iterate each device directory to be created            --- */
        /*****************************************************************************/

        for (
            device_name_opt,
            files_and_paths_vec
        ) in &devices_files_and_paths.file_tree {

            let mut indent_level: usize = 0;

            let do_create_device_subdirs = device_count_for_date > 1 && device_name_opt.is_some();

            // If there's more than one device, attach device dir to destination path, otherwise ignore devices
            let device_destination_path = if do_create_device_subdirs {
                // This is safe to unwrap, since we've already checked the device is_some
                let dir_name = device_name_opt.clone().unwrap();

                // Attach device name as a new subdirectory to the current target path
                let device_path = date_destination_path.join(dir_name.clone()); // we only need clone here to be able to print it out later

                // Print device dir name
                if is_dry_run {
                    // If dry run, increase indent for subsequent files
                    indent_level += 1;

                    // Add tree indents and padding to dir name
                    let _indented_dir_name: String = indent_string(0, format!("[{}] ", dir_name));

                    let padded_dir_name = RightPadding::dot(
                        _indented_dir_name,
                        // safe to unwrap for dry runs
                        dir_padding_width.unwrap());

                    // Check restrictions - if target exists
                    let target_dir_exists = dry_run_check_target_exists(&device_path);

                    // Print everything together
                    println!("{} {}", padded_dir_name, target_dir_exists);
                }

                device_path
            } else {
                date_destination_path.clone()
            };

            // Create subdir path
            if !is_dry_run {
                create_subdir_if_required(&device_destination_path, &args, &mut stats);
            }

            /*****************************************************************************/
            /* --- Iterate each file in a device directory and print or copy/move it --- */
            /*****************************************************************************/

            for file in files_and_paths_vec {

                // Attach filename to the directory path
                let mut file_destination_path = device_destination_path.clone()
                    .join(file.get_file_name_ref());

                let (padded_filename,
                    op_separator,
                    padded_path,
                    op_status
                ) = {

                    // need this space after the filename so there's a gap until the padding starts
                    let _filename_string = format!("{} ", &file.file_name.to_str().unwrap());

                    let _stripped_target_path = file_destination_path.strip_prefix(&args.target_dir).unwrap().display().to_string();
                    let padded_path = RightPadding::dot(
                        format!("{} ", _stripped_target_path),
                        // add +1 for the space added to the right of _stripped_target_path
                        new_dir_tree.max_path_len + 1);

                    // Check files and print result in this format:
                    //  └── DSC_0002.JPG ---> 2017.03.12\DSC_0002.JPG... file will be copied
                    if is_dry_run {

                        // Check restrictions - file exist or read only
                        let file_restrictions = dry_run_check_file_restrictions(&file.file_path, &file_destination_path, args);

                        // Add tree indents and dry run padding (normal dashes) to file name
                        let _indented_filename = indent_string(indent_level, _filename_string);
                        let padded_filename = RightPadding::dash(
                            _indented_filename,
                            // add +1 for the space added to the right of filename_string
                            new_dir_tree.max_filename_len + 1);

                        // Return everything to be printed
                        (padded_filename, SEPARATOR_DRY_RUN, padded_path, file_restrictions)

                    // Copy/move files then print result in this format:
                    // DSC_0002.JPG ───> 2017.03.12\DSC_0002.JPG... ok
                    } else {

                        // Copy/move file
                        let file_copy_status = copy_file_if_not_exists(
                            &file,
                            &mut file_destination_path,
                            &args, &mut stats);

                        // Add copy/move padding (em dashes) to file name
                        let padded_filename = RightPadding::em_dash(
                            _filename_string,
                            // add +1 for the space added to the right of filename_string
                            new_dir_tree.max_filename_len + 1);

                        // Return everything to be printed
                        (padded_filename, SEPARATOR_COPY_MOVE, padded_path, file_copy_status)
                    }
                };

                // Print operation status
                println!("{file}{op_separator} {path}{status_separator} {status}",
                         file=padded_filename,
                         op_separator=op_separator,
                         path=padded_path,
                         status_separator=SEPARATOR_STATUS,
                         status=op_status);
            } // end loop files
        } // end loop device dirs
    } // end loop date dirs
}

/// Read a directory path and return a string signalling if the path exists
fn dry_run_check_target_exists(path: &PathBuf) -> String {
    if path.exists() {
        String::from("[target folder exists, will be skipped]")
    } else {
        String::from("[new folder will be created]")
    }
}

/// Read a path and return a string signalling copy/move restrictions:
/// * in both cases, check if the source file exists - no copy will take place
/// * in both cases, check if the target file exists - file will be skipped
/// * if the is a move, check if the source file is read-only and can't be moved (only copied)
fn dry_run_check_file_restrictions(source_path: &PathBuf, target_path: &PathBuf, args: &CliArgs) -> String {

    if source_path.exists() {

        if target_path.exists() {
            ColoredString::orange("target file exists, will be skipped")
        } else if args.copy_not_move {
          ColoredString::green("file will be copied")

        } else {
            match source_path.metadata() {
                Ok(metadata) => {
                    let is_read_only = metadata.permissions().readonly();
                    if !args.copy_not_move && is_read_only {
                        ColoredString::red("source is read only, file will be copied")
                    } else {
                        ColoredString::green("file will be moved")
                    }
                },
                Err(e) => {
                    let err_status = format!("error reading metadata: {}", e.to_string());
                    ColoredString::red(err_status.as_str())
                }
            }
        }

    } else {
        ColoredString::red("source file does not exist")
    }
}

fn ask_for_confirmation() -> ConfirmationType {
    println!("{}",
             // TODO 5f: replace '\n' with system newlines
             ColoredString::magenta(
                 "OK to proceed? Type one of the options then press Enter:\n\
                 • 'y' or 'yes' to continue\n\
                 • 'n' or 'no' to cancel\n\
                 • 'd' or 'dry' to do a dry run"));
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
) -> String {

    if destination_path.exists() {
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
        ColoredString::orange("already exists")

    } else {

        let copy_result = fs::copy(file.get_file_path_ref(), &destination_path);

        match copy_result {

            // File creation was successful
            Ok(_) => {

                // If this is a MOVE, delete the source file after a successful copy and append status
                let (_delete_failed_opt, delete_result_str) = if !args.copy_not_move {

                    let delete_result = fs::remove_file(file.get_file_path_ref());

                    match delete_result {
                        Ok(_) =>
                            (Some(false), String::from(" (source file removed)")),
                        Err(e) => {
                            if DBG_ON { eprintln!("File delete error: {:?}: ERROR {:?}", file.get_file_path_ref(), e) };
                            stats.inc_error_file_delete();
                            (Some(true), ColoredString::red(
                                format!(" (error removing source: {:?})", e.description()).as_str()))
                        }
                    }
                // This is just a COPY operation, there's no delete result
                } else {
                    (None, String::from(""))
                };

                // Record stats for copied or moved files. Pay special attention to cases when the operation
                // is a move, the target file was created, but the source file was not deleted
                // If operation is a move, the delete_failed is *defined* and *true* if the deletion failed
                match file.file_type {
                    FileType::Image   =>
                        if args.copy_not_move || _delete_failed_opt.unwrap_or(false) { stats.inc_img_copied() } else { stats.inc_img_moved() },
                    FileType::Video   =>
                        if args.copy_not_move || _delete_failed_opt.unwrap_or(false) { stats.inc_vid_copied() } else { stats.inc_vid_moved() },
                    // don't record any stats for this, shouldn't get one here anyway
                    FileType::Unknown =>()
                }

                format!("{}{}",
                        ColoredString::green("ok"),
                        delete_result_str)
            },

            // Could not create target file, log error and don't even attempt to delete source
            Err(err) => {
                eprintln!("File copy error: {:?}: ERROR {:?}", file.get_file_path_ref(), err);
                // TODO 5c: log error info
                stats.inc_error_file_create();
                ColoredString::red("ERROR")
            }
        }
    }
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
            // create subdirs along the path as required
            // recursive + create doesn't return Err if dir exists
            .recursive(true)
            .create(target_subdir);

        match subdir_creation {
            Ok(_) => {
                stats.inc_dirs_created();
                println!();
                println!("{}",
                         ColoredString::bold_white(
                             format!("[Created folder {}]",
                            target_subdir.strip_prefix(&args.target_dir).unwrap().display()).as_str()));
            },
            Err(e) => {
                // TODO 2f: handle dir creation fail
                stats.inc_error_dir_create();
                println!("Failed to create folder {}: {:?}",
                         target_subdir.strip_prefix(&args.target_dir).unwrap().display(),
                         e.kind())
            }
        }
    };
}

/// Read metadata and return the file's modified time in YYYY-MM-DD format
/// This is the operating systemțs Date Modified: the time that any application or
/// the camera or the operating system itself modified the file.
/// See also [read_exif_date_and_device()]
fn get_system_modified_date(file_metadata: &Metadata) -> Option<String> {
    file_metadata.modified().map_or(None, |system_time| {
        let datetime: DateTime<Utc> = system_time.into(); // 2021-06-05T16:26:22.756168300Z
        Some(datetime.format(DATE_DIR_FORMAT).to_string())
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

/// Read a String in standard EXIF format "YYYY:MM:DD HH:MM:SS"
/// and try to parse it into the date format for our directories: "YYYY.MM.DD"
fn parse_exif_date(date_str: String) -> Option<String> {
    let parsed_date_result = NaiveDateTime::parse_from_str(date_str.as_str(), "%Y:%m:%d %H:%M:%S");
    match parsed_date_result {
        Ok(date) => {
            let formatted_date = date.format(DATE_DIR_FORMAT).to_string();
            Some(formatted_date)
        }
        Err(err) => {
            if DBG_ON { println!("> could not parse EXIF date {}: {:?}", date_str, err) }
            None
        }
    }
}

fn read_exif_date_and_device(file: &DirEntry) -> ExifDateDevice {

    // Create an empty Exif object and set values after reading EXIF data
    let mut file_exif = ExifDateDevice{
        date_original: None,
        date_time: None,
        camera_model: None
    };

    // TODO 5d: handle this unwrap
    // Return early if this is not a file, there's no device name to read
    if file.metadata().unwrap().is_dir() {
        return file_exif
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
                            file_exif.camera_model = Some(tag_value)
                        },

                        // Comments based on https://feedback-readonly.photoshop.com/conversations/lightroom-classic/date-time-digitized-and-date-time-differ-from-date-modified-and-date-created/5f5f45ba4b561a3d425c6f77

                        // EXIF:DateTime: When photo software last modified the image or its metadata.
                        // Operating system Date Modified: The time that any application or the camera or
                        // operating system itself modified the file.
                        // The String returned by rexif has the standard EXIF format "YYYY:MM:DD HH:MM:SS"
                        ExifTag::DateTime => {
                            let tag_value = exif_entry.value.to_string();
                            file_exif.date_time = parse_exif_date(tag_value);
                        }

                        // EXIF:DateTimeOriginal: When the shutter was clicked. Windows File Explorer will display it as Date Taken.
                        ExifTag::DateTimeOriginal => {
                            let tag_value = exif_entry.value.to_string();
                            file_exif.date_original = parse_exif_date(tag_value);
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
            if DBG_ON {
                println!("{} could not read EXIF for {:?}: {}", ColoredString::warn_arrow(), file.file_name(), e.to_string());
            }
        }
    }

    return file_exif;
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