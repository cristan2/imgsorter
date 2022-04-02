use std::path::{PathBuf, Path};
use std::{fs, io, fmt};
use std::cmp::max;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt::Formatter;
use std::iter::FromIterator;
use std::fs::{DirEntry, DirBuilder, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use rexif::{ExifTag, ExifResult};
use chrono::{DateTime, NaiveDateTime, Utc};
use filesize::PathExt;

use imgsorter::config::{DBG_ON, Args};
use imgsorter::utils::*;

const DEFAULT_NO_DATE_STR: &'static str = "no date";
const DATE_DIR_FORMAT: &'static str = "%Y.%m.%d";

/// Convenience wrapper over a map holding all files for a given device
/// where the string representation of the optional device is the map key
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
/// The outer map key is the date representation
/// The inner map key is an Optional device name
/// Use BTreeMap's to have the keys sorted
/// ```
/// [target_dir]          // wrapper struct
///  └─ [date_dir]        // wrapped top-level map; key of type String
///  │   └─ [device_dir]  // inner map; key of type Option<String>
///  │   │   └─ file.ext  // inner map; value is Vec of supported files
///  │   │   └─ file.ext
///  │   └─ device_dir
///  │   │   └─ ...
///  └─ [assorted]
///  │   └─ single.file
/// ```
/// Additionally, the struct
struct TargetDateDeviceTree {
    dir_tree: BTreeMap<String, DeviceTree>,
    // max_filename_len: usize,
    // max_source_path_len: usize,
    // computed at the end
    max_target_path_len: usize
}

/// Just output a simple list of filenames for now
impl fmt::Display for TargetDateDeviceTree {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let file_names: Vec<String> = self.dir_tree
            .iter()
            .map(|(date_dir, device_tree)| {

                let date_files = device_tree.file_tree
                    .iter()
                    .flat_map(|(device_dir, files)| {
                        let device_files = files.iter()
                            .map(|file|
                                format!("{:?} | {}", device_dir, file.file_path.display().to_string()))
                            .collect::<Vec<String>>();
                        device_files
                    })
                    .collect::<Vec<String>>();

                format!("[{}]\n{}", date_dir, date_files.join("\n"))
            })
            .collect();
        write!(f, "{}", file_names.join("\n"))
    }
}

impl TargetDateDeviceTree {
    fn new() -> TargetDateDeviceTree {
        TargetDateDeviceTree {
            dir_tree: BTreeMap::new(),
            // max_filename_len: 0,
            // max_source_path_len: 0,
            max_target_path_len: 0,
        }
    }

    /// Iterate all files in this this map and move all files which are in a directory with
    /// less than args.min_files_per_dir into a new separate directory called [DEFAULT_ONEOFFS_DIR_NAME].
    ///
    /// In practice, this should avoid creating date dirs which contain a single file. Instead,
    /// all such one-offs will be placed together in a single directory.
    ///
    /// Returns a new [DateDeviceTree] object
    fn isolate_single_images(mut self, args: &Args) -> Self {

        // Don't bother doing anything if we don't have at least a threshold of 1
        if args.min_files_per_dir <= 0 {
            return self
        }

        let _has_single_device = |device_tree: &DeviceTree| device_tree.file_tree.keys().len() < 2;

        let _has_minimum_files = |device_tree: &DeviceTree| {
            let all_files_names  = device_tree
                .file_tree
                .values()
                .flat_map(|files|
                    files.iter().map(|f| f.file_name.clone()))
                .collect::<Vec<_>>();

            let all_files_unique: HashSet<&OsString> = HashSet::from_iter(all_files_names.iter());
            let all_files_count = all_files_unique.len();
            all_files_count <= args.min_files_per_dir as usize
        };

        let has_oneoff_files = |device_tree: &DeviceTree| {
            _has_single_device(&device_tree) && _has_minimum_files(&device_tree)
        };

        // TODO 5h: this is inefficient, optimize to a single iteration and non-consuming method
        let mut devices_tree: BTreeMap<String, DeviceTree> = BTreeMap::new();
        let mut oneoff_files: Vec<SupportedFile> = Vec::new();

        self.dir_tree
            .into_iter()
            .for_each(|(device_dir, device_tree)| {
                // Move single files from the current date dir to a separate dir,
                // which will be joined again later under a different key
                if has_oneoff_files(&device_tree) {

                    // TODO 6g handle max_len and possible file duplicates
                    device_tree.file_tree
                        .into_iter()
                        .for_each(|(_, src_files)| oneoff_files.extend(src_files));

                // keep the existing date-device structure
                } else {
                    devices_tree.insert(device_dir, device_tree);
                }
            });

        let mut oneoffs_tree = DeviceTree::new();
        oneoffs_tree.file_tree.insert(None, oneoff_files);
        devices_tree.insert(args.oneoffs_dir_name.clone(), oneoffs_tree);

        self.dir_tree = devices_tree;

        self
    }

    /// Find the maximum length of the path string that may be present in the output
    /// This can only be computed after the tree has been filled with devices and files
    /// because of the requirement to only create device subdirs if there are at least 2 devices
    /// Resulting value covers two cases:
    /// - there's at least one date dir with >1 devices subdirs: compute path length to include `date/device_name/file_name`
    /// - there's no date dir with >1 devices: compute path length to include `date/file_name`
    fn compute_max_path_len(&mut self, padder: &Padder) {
        let max_date_dir_path_len = &self.dir_tree.iter()
            // filter only date dirs with at least 2 devices
            .filter(|(_, device_tree)| device_tree.file_tree.keys().clone().len() > 1 )
            // now search all devices for the max path len
            .map(|(_, device_tree)| device_tree.max_dir_path_len)
            .max();

        if max_date_dir_path_len.is_some() {
            // add +1 for the length of the separator between dirs and filename
            self.max_target_path_len = max_date_dir_path_len.clone().unwrap() + 1 + padder.source_file_max_len;
        } else {
            // add +10 for the length of date dirs, e.g. 2016.12.29
            // add +1 for the length of the separator between date and filename
            self.max_target_path_len = 10 + 1 + padder.source_file_max_len;
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
    files_count_total: i32,
    file_size_total: u64,
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
    time_fetch_dirs: Duration,
    time_parse_files: Duration,
    time_write_files: Duration,
    time_total: Duration
}

impl FileStats {
    pub fn new() -> FileStats {
        FileStats {
            files_count_total: 0,
            file_size_total: 0,
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
            time_fetch_dirs: Duration::new(0, 0),
            time_parse_files: Duration::new(0, 0),
            time_write_files: Duration::new(0, 0),
            time_total: Duration::new(0, 0)
        }
    }

    pub fn inc_files_total(&mut self, count: usize) { self.files_count_total += count as i32}
    pub fn inc_files_size(&mut self, size: u64) { self.file_size_total += size }
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
    pub fn set_time_fetch_dirs(&mut self, elapsed: Duration) { self.time_fetch_dirs = elapsed }
    pub fn set_time_parse_files(&mut self, elapsed: Duration) { self.time_parse_files = elapsed }
    pub fn set_time_write_files(&mut self, elapsed: Duration) { self.time_write_files = elapsed }
    pub fn set_time_total(&mut self, elapsed: Duration) { self.time_total = elapsed }

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

    pub fn print_stats(&self, args: &Args) {
        let general_stats = format!(
"---------------------------------
Total files:             {total}
Total size:              {size}
---------------------------------
Images moved:            {img_move}
Images copied:           {img_copy}
Images skipped:          {img_skip}
Videos moved:            {vid_move}
Videos copied:           {vid_copy}
Videos skipped:          {vid_skip}
Directories ignored:     {dir_ignore}
Directories created:     {dir_create}
Unknown files skipped:   {f_skip}
---------------------------------
File create errors:      {fc_err}
File delete errors:      {fd_err}
Directory create errors: {dc_err}
---------------------------------
Time fetching folders:   {tfetch_dir}s
Time parsing files:      {tparse_file}s
Time writing files:      {twrite_file}s
---------------------------------
Total time taken:        {t_total}s
---------------------------------",
                                    total=FileStats::color_if_non_zero(self.files_count_total, OutputColor::Neutral),
                                    size=ColoredString::bold_white(get_file_size_string(self.file_size_total).as_str()),

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

                                    tfetch_dir=ColoredString::bold_white(format!("{}:{}",
                                                                                 self.time_fetch_dirs.as_secs(),
                                                                                 LeftPadding::zeroes3(self.time_fetch_dirs.subsec_millis())).as_str()),
                                    tparse_file=ColoredString::bold_white(format!("{}:{}",
                                                                                  self.time_parse_files.as_secs(),
                                                                                  LeftPadding::zeroes3(self.time_parse_files.subsec_millis())).as_str()),
                                    twrite_file=ColoredString::bold_white(format!("{}:{}",
                                                                                  self.time_write_files.as_secs(),
                                                                                  LeftPadding::zeroes3(self.time_write_files.subsec_millis())).as_str()),
                                    t_total=ColoredString::bold_white(format!("{}:{}",
                                                                                  self.time_total.as_secs(),
                                                                                  LeftPadding::zeroes3(self.time_total.subsec_millis())).as_str()),
        );

        println!("{}", general_stats);

        if self.files_count_total == self.unknown_skipped {
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
    // index of the vec holding the files in the original source dir of this file
    // this is used to detect duplicates across multiple source dirs when doing dry runs
    source_dir_index: usize
}

// TODO 5e: find better name
impl SupportedFile {
    pub fn parse_from(dir_entry: DirEntry, source_index: usize) -> SupportedFile {
        let _extension = get_extension(&dir_entry);
        let _file_type = get_file_type(&_extension);
        let _metadata = dir_entry.metadata().unwrap();

        let mut _empty_exif = ExifDateDevice {
            date_original: None,
            date_time: None,
            camera_model: None
        };

        let _exif_data = match _file_type {
            // It's much faster if we only try to read EXIF for image files
            FileType::Image =>
                read_exif_date_and_device(&dir_entry, _empty_exif),
            _ =>
                _empty_exif
        };

        // Read image date - prefer EXIF tags over system date
        let _image_date = _exif_data.date_original
            .unwrap_or(_exif_data.date_time
                .unwrap_or(get_system_modified_date(&_metadata)
                    .unwrap_or(DEFAULT_NO_DATE_STR.to_string())));

        SupportedFile {
            file_name: dir_entry.file_name(),
            file_path: dir_entry.path(),
            file_type: _file_type,
            extension: _extension,
            date_str: _image_date,
            metadata: _metadata,
            device_name: _exif_data.camera_model,
            source_dir_index: source_index
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

fn main() -> Result<(), std::io::Error> {

    let mut stats = FileStats::new();

    let mut padder = Padder::new();

    /*****************************************************************************/
    /* ---                     Read or set args                              --- */
    /*****************************************************************************/

    let mut args = Args::new_from_toml("imgsorter.toml")?;

    if DBG_ON {
        dbg!(&args);
    }


    /*****************************************************************************/
    /* ---                        Read source dirs                           --- */
    /*****************************************************************************/

    if args.source_recursive {

        if DBG_ON { println!("> Fetching source directories list recursively..."); }
        let time_fetching_dirs = Instant::now();

        let new_source_dirs = walk_source_dirs_recursively(&args);
        if new_source_dirs.is_empty() {
            // TODO replace with Err
            panic!("Source folders are empty or don't exist");
        } else {
            args.source_dir = new_source_dirs;
        }

        stats.set_time_fetch_dirs(time_fetching_dirs.elapsed());
    }

    /*****************************************************************************/
    /* ---                        Read source files                          --- */
    /*****************************************************************************/

    // TODO 6f: handle path not exists
    // TODO 5g: instead of Vec<Vec<DirEntry>>, return a `SourceDirTree` struct
    // which wraps the Vec's but contains additional metadata, such as no of files or total size
    // Read dir contents and filter out error results
    let source_contents = args.source_dir.clone()
        .iter()
        .filter_map(|src_dir|
            read_supported_files(src_dir, &mut stats, &mut args).ok())
        .collect::<Vec<_>>();


    /*****************************************************************************/
    /* ---                 Print options before confirmation                 --- */
    /*****************************************************************************/

    {
        let total_source_files: usize = source_contents.iter()
            .map(|dir|dir.len())
            .sum();

        let copy_status = if args.copy_not_move {
            ColoredString::orange("copied:")
        } else {
            ColoredString::red("moved: ")
        };

        // TODO 6f: check paths exist
        // Build the string used for printing source directory name(s) before confirmation
        let source_dirs = {
            let source_dir_str = String::from("Source directory:   ");

            match args.source_dir.len() {
                0 =>
                    format!("{}{}", source_dir_str, ColoredString::red("No source dirs specified")),
                1 =>
                    format!("{}{}", source_dir_str, args.source_dir[0].display().to_string()),
                _ => {
                    let spacing_other_lines = " ".repeat(source_dir_str.chars().count());
                    args.source_dir
                        .iter()
                        .enumerate()
                        .map(|(index, src_path)| {
                            let _first_part = if index == 0 {&source_dir_str} else {&spacing_other_lines};
                            format!("{}{}. {}", _first_part, index, &src_path.display().to_string()) })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
        };

        println!("===========================================================================");
        // TODO This would only be relevant if we're saving any files or reading config
        // println!("Current working directory: {}", &args.cwd.display());
        println!("{}", source_dirs);
        println!("Target directory:   {}", &args.target_dir.display());
        println!("Files to be {} {}", copy_status, total_source_files);
        println!("===========================================================================");
        // TODO 1f: print all options for this run?
    }

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

    let time_processing = Instant::now();

    println!("---------------------------------------------------------------------------");
    println!();


    /*****************************************************************************/
    /* ---        Parse source files and copy/paste or dry run them          --- */
    /*****************************************************************************/

    // Extract unique file names across all source directories.
    // Useful only for dry run statuses
    let source_unique_files = get_source_unique_files(&source_contents, &args);

    // TODO prefilter for Images and Videos only
    // Iterate files, read modified date and create subdirs
    // Copy images and videos to subdirs based on modified date
    let time_parsing_files = Instant::now();
    let mut target_dir_tree = parse_dir_contents(source_contents, &args, &mut stats, &mut padder);

    stats.set_time_parse_files(time_parsing_files.elapsed());

    let time_writing_files = Instant::now();
    if !target_dir_tree.dir_tree.is_empty() {
        // Iterate files and either copy/move to subdirs as necessary
        // or do a dry run to simulate a copy/move pass
        write_target_dir_files(&mut target_dir_tree, source_unique_files, &args, &mut stats, &mut padder);
    }

    // Record time taken
    // Dirs fetching occurs before confirmation, while start time starts after confirmation
    stats.set_time_write_files(time_writing_files.elapsed());
    stats.set_time_total(time_processing.elapsed() + stats.time_fetch_dirs);

    // Print final stats
    stats.print_stats(&args);

    Ok(())
}

fn walk_source_dirs_recursively(args: &Args) -> Vec<PathBuf> {

    fn walk_dir(
        source_dir: PathBuf,
        vec_accum: &mut Vec<PathBuf>
    ) -> Result<(), std::io::Error> {

        if DBG_ON {
            println!("> Reading {:?}...", &source_dir);
        }

        let subdirs: Vec<DirEntry> = fs::read_dir(&source_dir)?
            .into_iter()
            .filter_map(|s| s.ok())
            .filter(|entry| entry.path().is_dir())
            .collect::<Vec<_>>();

        vec_accum.push(source_dir);            

        if !subdirs.is_empty() {
            subdirs
                .iter()
                .for_each(|dir_entry| {
                    let _ = walk_dir(dir_entry.path(), vec_accum); 
                });
        };

        Ok(())
    }

    let mut new_source_dirs = Vec::new();

    args.source_dir.clone()
        .into_iter()
        .for_each(|d| {
            walk_dir(d, &mut new_source_dirs).ok();
        });

    new_source_dirs
}

/// For dry runs over multiple source dirs we want to show if there are duplicate files across all sources.
/// This method iterates over all filenames in the source dirs, extracting a set for each dir.
/// Then it iterates all sets, checking each one against all previous ones and keeps only
/// unique elements, ensuring duplicate filenames are progressively removed.
fn get_source_unique_files(
    source_dir_contents: &Vec<Vec<DirEntry>>,
    args: &Args
) -> Option<Vec<HashSet<OsString>>> {

    // This method is only useful for dry runs, return early otherwise
    if !args.dry_run {
        return None
    }

    let sets_of_files = source_dir_contents.iter().map(|src_dir|
        src_dir.iter()
            .map(|src_entry| src_entry.file_name())
            .collect::<HashSet<_>>())
        .collect::<Vec<_>>();

    if DBG_ON { print_sets_with_index("source file sets before filtering", &sets_of_files); }

    let all_unique_files = sets_of_files
        .iter().enumerate()
        .map(|(curr_ix, _)|
            // keep_unique_across_sets(&sets_of_files, curr_ix))
            keep_unique_across_sets(&sets_of_files[0..=curr_ix]))
        .collect::<Vec<_>>();

    if DBG_ON { print_sets_with_index("source file sets after filtering", &all_unique_files); }

    Option::from(all_unique_files)
}

/// Read contents of source dir and filter out directories or those which failed to read
fn read_supported_files(
    source_dir: &PathBuf,
    stats: &mut FileStats,
    args: &mut Args
) -> Result<Vec<DirEntry>, std::io::Error> {

    // TODO 5d: handle all ?'s
    let dir_entries = fs::read_dir(source_dir)?
        .into_iter()
        .filter_map(|entry| entry.ok());

    // filter out any source subdirectories...
    let filtered_entries = if args.source_recursive {
        dir_entries
            .filter(|entry| entry.path().is_file())
            .collect::<Vec<DirEntry>>()

    // ...but record stats if "source_recursive" is not enabled
    } else {
        dir_entries
            .filter(|entry| 
                if entry.path().is_file() {
                    true
                } else {
                    if DBG_ON {
                        println!("Recursive option is off, skipping subfolder {:?} in {:?}", entry.file_name(), source_dir.file_name().unwrap());
                    }
                    stats.inc_dirs_ignored();
                    false
                }
            )
            .collect::<Vec<DirEntry>>()
        };
    
    Ok(filtered_entries)
}

/// Read directory and parse contents into supported data models
fn parse_dir_contents(
    source_dir_contents: Vec<Vec<DirEntry>>,
    args: &Args,
    stats: &mut FileStats,
    padder: &mut Padder
) -> TargetDateDeviceTree {

    let mut new_dir_tree: TargetDateDeviceTree = TargetDateDeviceTree::new();

    // TODO 5g: this should already be available from source_dir_contents metadata
    let total_no_files: usize = source_dir_contents.iter().map(|vec|vec.len()).sum();

    stats.inc_files_total(total_no_files);

    let mut count_so_far = 0;

    for (source_ix, source_dir) in source_dir_contents.into_iter().enumerate() {

        let time_parsing_dir = Instant::now();

        let current_file_count = source_dir.len();

        print_progress(format!("[{}/{}] Parsing {} files from {}... ",
                               count_so_far,
                               total_no_files,
                               current_file_count,
                               args.source_dir[source_ix].display()));

        // Parse each file into its internal representation and add it to the target tree
        for entry in source_dir {

            let current_file: SupportedFile = SupportedFile::parse_from(entry, source_ix);

            // Build final target path for this file
            match current_file.file_type {
                FileType::Image | FileType::Video => {
                    let file_date = current_file.date_str.clone();
                    let file_device = current_file.device_name.clone();

                    // TODO 5i: replace these with single method in DateDeviceTree
                    // Attach file's date as a new subdirectory to the current target path
                    let devicetree_for_this_date = {
                        new_dir_tree
                            .dir_tree
                            .entry(file_date)
                            .or_insert(DeviceTree::new())
                    };

                    // TODO 5i: replace these with single method in DeviceTree
                    let all_files_for_this_device = {
                        devicetree_for_this_date
                            .file_tree
                            .entry(file_device)
                            .or_insert(Vec::new())
                    };

                    // Store the string lengths of the file name and path for padding in stdout
                    // let _filename_len = String::from(current_file.file_name.clone().to_str().unwrap()).chars().count();
                    // let _source_path_len = current_file.file_path.display().to_string().chars().count();

                    let _device_name_len = current_file.device_name.clone().map(|d| d.chars().count()).unwrap_or(0);
                    let _date_name_str = &current_file.date_str.chars().count();
                    // add +1 for each path separator character
                    let total_target_path_len = _date_name_str + 1 + _device_name_len;

                    // padder.max ...
                    padder.set_max_source_filename_from_str(current_file.file_name.clone().to_str().unwrap());
                    padder.set_max_source_path(get_string_char_count(current_file.file_path.display().to_string()));
                    // new_dir_tree.max_filename_len = max(new_dir_tree.max_filename_len, _filename_len);
                    // new_dir_tree.max_source_path_len = max(new_dir_tree.max_source_path_len, _source_path_len);
                    devicetree_for_this_date.max_dir_path_len = max(devicetree_for_this_date.max_dir_path_len, total_target_path_len);

                    // Add file to dir tree
                    all_files_for_this_device.push(current_file);
                }

                FileType::Unknown => {
                    stats.inc_unknown_skipped();
                    if DBG_ON {
                        println!("Skipping unknown file {:?}", current_file.get_file_name_ref())
                    }
                }
            }
        }

        // Record progress
        count_so_far += current_file_count;

        print_progress(format!("done ({}.{} sec)",
                               time_parsing_dir.elapsed().as_secs(),
                               LeftPadding::zeroes3(time_parsing_dir.elapsed().subsec_millis())));
        println!();
    }

    // This is a consuming call for now, so needs reassignment
    new_dir_tree = new_dir_tree.isolate_single_images(args);

    // The max path length can only be computed after the tree has been filled with devices and files
    // because of the requirement to only create device subdirs if there are at least 2 devices
    new_dir_tree.compute_max_path_len(&padder);

    return new_dir_tree;
}

fn write_target_dir_files(
    // The target tree representation of files to be copied/moved
    new_dir_tree: &mut TargetDateDeviceTree,
    // For dry runs, this represents a vector of unique files per each source dir
    source_unique_files: Option<Vec<HashSet<OsString>>>,
    args: &Args,
    mut stats: &mut FileStats,
    padder: &mut Padder
) {

    let is_dry_run = args.dry_run;

    // Dry runs will output a dir-tree-like structure, so add the additional
    // indents and markings to the max length to be taken into account when padding
    if is_dry_run {
        // TODO need to pre-calculate max-depth length
        // TODO FILE_TREE_INDENT is not required when there's only one level (i.e. one single device throughout)
        // let _extra_indents_len =
        //     String::from(FILE_TREE_INDENT).chars().count()
        //         + String::from(FILE_TREE_ENTRY).chars().count();

        padder.add_extra_source_chars_from_str(FILE_TREE_INDENT);
        padder.add_extra_source_chars_from_str(FILE_TREE_ENTRY);

        // // TODO cand this be a single field instead of two?
        // if args.has_multiple_sources() {
        //     new_dir_tree.max_source_path_len = new_dir_tree.max_source_path_len + _extra_indents_len
        // } else {
        //     new_dir_tree.max_filename_len = new_dir_tree.max_filename_len + _extra_indents_len
        // }
    } else {
        println!();
        let start_status = format!("Starting to {} files...", { if args.copy_not_move {"copy"} else {"move"}} );
        println!("{}", ColoredString::bold_white(start_status.as_str()));
        println!();
    }

    let dir_padding_width = {
        if is_dry_run {
            // TODO can be calculated and set earlier, not here
            // let _source_len = if args.has_multiple_sources() {
            //     new_dir_tree.max_source_path_len
            // } else {
            //     new_dir_tree.max_filename_len
            // };

            // TODO temporary
            let _source_len = padder.get_total_max_source_len(args.has_multiple_sources());

            // TODO to be converted into Padder::_total_max_len
            let _total_padding_width = {
                _source_len
                    + 1 // add +1 for the gap between a filename and its padding
                    + SEPARATOR_DRY_RUN.chars().count()
                    + new_dir_tree.max_target_path_len
                    + SEPARATOR_STATUS.chars().count()
                    + 1 // add +1 for the gap between a path and its padding
                    + 1 // add +1 for the gap between a path and the operation status
            };

            // TODO 5h: fix padding
            // Also print headers now
            {
                // TODO Padding::format_header_source
                let source_padding = RightPadding::space(
                    String::from("SOURCE PATH"),
                    _source_len
                        + 1 // add +1 for the gap between a filename and its padding
                        + SEPARATOR_DRY_RUN.chars().count()
                );

                // TODO Padding::format_header_target
                let target_padding = RightPadding::space(
                    String::from("TARGET FILE"), new_dir_tree.max_target_path_len);

                // TODO Padding::format_header_separator
                let heading = "-".repeat(_total_padding_width);

                println!("{}", &heading);
                println!("{}{}", source_padding, target_padding);
                println!("{}", heading);
            }

            Some(_total_padding_width)
        } else {
            None
        }
    };


    /*****************************************************************************/
    /* ---             Iterate each date directory to be created             --- */
    /*****************************************************************************/

    for (date_dir_name, devices_files_and_paths) in &new_dir_tree.dir_tree {
        let device_count_for_date = devices_files_and_paths.file_tree.keys().len();

        // Get a total sum of file counts and file size in a single iteration
        let (file_count_for_date, file_size_for_date) = devices_files_and_paths.file_tree.iter()
            .fold((0, 0), |(accum_count, accum_size), (_, files_and_paths)|
                (
                    accum_count + files_and_paths.len(),
                    accum_size + get_files_size(files_and_paths)
                ));
        stats.inc_files_size(file_size_for_date);

        // Attach file's date as a new subdirectory to the target path
        let date_destination_path = args.target_dir.clone().join(date_dir_name);

        if is_dry_run {

            let _device_count_str = if device_count_for_date == 1 {"device"} else {"devices"};
            let _file_count_str = if file_count_for_date == 1 {"file"} else {"files"};

            let _dir_name_with_device_status = {
                format!(
                    "[{dirname}] ({devicecount:?} {devicestr}, {filecount:?} {filestr}, {filesize}) ",
                    dirname = date_dir_name.clone(),
                    devicecount = device_count_for_date,
                    devicestr = _device_count_str,
                    filecount = file_count_for_date,
                    filestr = _file_count_str,
                    filesize = get_file_size_string(file_size_for_date))
            };

            // TODO replace with Padding::format_date_dir
            let padded_dir_name = RightPadding::dot(
                _dir_name_with_device_status,
                // safe to unwrap for dry runs
                dir_padding_width.unwrap());

            // Check restrictions - if target exists
            let target_dir_exists = dry_run_check_target_exists(&date_destination_path);

            // Print everything together
            println!("{} {}", padded_dir_name, target_dir_exists);
        }


        /*****************************************************************************/
        /* ---            Iterate each device directory to be created            --- */
        /*****************************************************************************/

        for (
            device_name_opt,
            files_and_paths_vec
        ) in &devices_files_and_paths.file_tree {

            let mut indent_level: usize = 0;

            // This condition helps prevent creating a redundant device subdir if
            // there's only a single Some("device") device (without any "None" device files)
            // Before                 After
            // ------                 -----
            // [date_dir]             [date_dir]
            //  └─ [device_dir]        |
            //      └─ file01.ext      └─ file01.ext
            //      └─ file02.ext      └─ file02.ext
            let has_at_least_one_distinct_device = device_count_for_date > 1 && device_name_opt.is_some();

            // This condition helps prevent creating a device subdir for a single file, if there's also
            // a "None" device with a single file. In practice, this is most likely to be a situation where
            // a picture taken with a camera (computed device is Some("device") based on EXIF) is sent
            // via whatsapp and would end up in a "Sent" folder without EXIF info (computed device is None)
            // Before                 After
            // ------                 -----
            // [date_dir]             [date_dir]
            //  └─ [device_dir]        |
            //  │   └─ file01.ext      └─ file01.ext
            //  └─ file02.ext          └─ file02.ext
            // TODO 2g: add more logic to this case and maybe skip copying the file without EXIF info
            let has_double_file = device_count_for_date == 2 && file_count_for_date == 2;

            let do_create_device_subdirs = has_at_least_one_distinct_device && !has_double_file;

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

                    // TODO replace this with Padder::format_device_dir() then delete _indented_dir_name above
                    let padded_dir_name = RightPadding::dot(
                        _indented_dir_name,
                        // safe to unwrap for dry runs
                        dir_padding_width.unwrap());

                    // Check restrictions - if target exists
                    let target_dir_status_check = dry_run_check_target_exists(&device_path);

                    // Print everything together
                    println!("{} {}", padded_dir_name, target_dir_status_check);
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
                    write_result
                ) = {

                    // TODO move to SupportedFile::getDisplayString()
                    // need this space after the filename so there's a gap until the padding starts
                    // let _filename_string = format!("{} ", &file.file_name.to_str().unwrap());
                    let _filename_string = if args.has_multiple_sources() {
                        format!("{} ", &file.file_path.display().to_string())
                    } else {
                        format!("{} ", &file.file_name.to_str().unwrap())
                    };

                    let _stripped_target_path = file_destination_path.strip_prefix(&args.target_dir).unwrap().display().to_string();
                    // TODO replace with Padder::format_target_path()
                    let padded_target_path = RightPadding::dot(
                        format!("{} ", _stripped_target_path),
                        // add +1 for the space added to the right of _stripped_target_path
                        new_dir_tree.max_target_path_len + 1);

                    // Check files and print result in this format:
                    //  └── DSC_0002.JPG ---> 2017.03.12\DSC_0002.JPG... file will be copied
                    if is_dry_run {

                        // Check restrictions - file exist or read only
                        let file_restrictions = dry_run_check_file_restrictions(&file, &file_destination_path, &source_unique_files, args);

                        // Add tree indents and dry run padding (normal dashes) to file name
                        // TODO replace with Padder::format_source_path()
                        let _indented_source_filename = indent_string(indent_level, _filename_string);
                        let padded_source_filename = RightPadding::dash(
                            _indented_source_filename,
                            // add +1 for the space added to the right of filename_string
                            // if args.has_multiple_sources() {new_dir_tree.max_source_path_len} else {new_dir_tree.max_filename_len}
                            padder.get_total_max_source_len(args.has_multiple_sources())
                            + 1);

                        // Return everything to be printed
                        (padded_source_filename, SEPARATOR_DRY_RUN, padded_target_path, file_restrictions)

                    // Copy/move files then print result in this format:
                    // DSC_0002.JPG ───> 2017.03.12\DSC_0002.JPG... ok
                    } else {

                        // Copy/move file
                        let file_write_status = copy_file_if_not_exists(
                            &file,
                            &mut file_destination_path,
                            &args, &mut stats);

                        // TODO Padder::???
                        // Add copy/move padding (em dashes) to file name
                        let padded_filename = RightPadding::em_dash(
                            _filename_string,
                            // if args.has_multiple_sources() {new_dir_tree.max_source_path_len} else {new_dir_tree.max_filename_len}
                            padder.get_total_max_source_len(args.has_multiple_sources())
                            // add +1 for the space added to the right of filename_string
                             + 1);

                        // Return everything to be printed
                        (padded_filename, SEPARATOR_COPY_MOVE, padded_target_path, file_write_status)
                    }
                };

                // Print operation status
                println!("{file}{op_separator} {path}{status_separator} {status}",
                         file=padded_filename,
                         op_separator=op_separator,
                         path=padded_path,
                         status_separator=SEPARATOR_STATUS,
                         status=write_result);
            } // end loop files
        } // end loop device dirs

        // leave some empty space before the date dir
        println!();
    } // end loop date dirs
}

/// Read file metadata and return size in bytes
fn get_files_size(files: &Vec<SupportedFile>) -> u64 {
    files
        .iter()
        .map(|file| {
            let f_path = &file.file_path;
            f_path.size_on_disk_fast(&file.metadata).ok().unwrap_or(0)
        })
        .sum()
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
/// * in both cases, if there are multiple source dirs, check if the file is present more than once - skip all duplicates
/// * if the is a move, check if the source file is read-only and can't be moved (only copied)
fn dry_run_check_file_restrictions(
    source_file: &SupportedFile,
    target_path: &PathBuf,
    source_unique_files: &Option<Vec<HashSet<OsString>>>,
    args: &Args
) -> String {

    // TODO 5d: Pre-filtering is not the best method to skip duplicate files.
    // It can fail for files with the same name in different directories, taken with different devices.
    // The alternative would be to store each file in a separate name during
    // the copy/move process and check each it against all previous ones.
    // If we find it, this means we're now dealing with a duplicate.

    // Check the index of unique files for the source dir of this file
    // If this set doesn't contain this file, then the file is a duplicate
    let is_source_unique = || {
      match source_unique_files {
          Some(source_dir_sets) => {
            let source_dir_index: usize = source_file.source_dir_index.clone();
            let source_dir_unique_files: &HashSet<OsString> = &source_dir_sets[source_dir_index];
            source_dir_unique_files.contains(&source_file.file_name)
          },
          None =>
            true
      }
    };

    if source_file.file_path.exists() {

        // Check if the target file exists

        // The order of checks matters - check for duplicates first, otherwise the reason
        // for skipping it will not be accurate. If the target file actually exists,
        // only the first of the duplicates should show as skipped for that reason.
        if !is_source_unique() {
            ColoredString::orange("duplicate source file, will be skipped")
        } else if target_path.exists() {
            ColoredString::orange("target file exists, will be skipped")
        } else if args.copy_not_move {
          ColoredString::green("file will be copied")

        } else {

            // Check if the source file can be deleted after copy

            match source_file.file_path.metadata() {
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
    args: &Args,
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
    args: &Args,
    stats: &mut FileStats
) {
    if target_subdir.exists() {
        if DBG_ON {
            println!("> target subdir exists: {}",
                     &target_subdir.strip_prefix(&args.target_dir).unwrap().display());
        }
    } else {
        // TODO 5x: same as fs::create_dir_all()
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
/// This is the operating system's Date Modified: the time that any application or
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

fn read_exif_date_and_device(
    file: &DirEntry,
    mut file_exif: ExifDateDevice
) -> ExifDateDevice {

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