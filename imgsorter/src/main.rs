use std::path::{PathBuf};
use std::{fs, io, fmt};
use std::cmp::max;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt::Formatter;
use std::iter::FromIterator;
use std::fs::{DirEntry, Metadata};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use filesize::PathExt;

use imgsorter::config::*;
use imgsorter::exif::*;
use imgsorter::utils::*;
use OutputColor::*;

/// Convenience wrapper over a map holding all files for a given device
/// where the string representation of the optional device is the map key
struct DeviceTree {
    file_tree: BTreeMap<DirEntryType, Vec<SupportedFile>>,
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
///  ├─ [date_dir]        // wrapped top-level map; key of type String
///  │   ├─ [device_dir]  // inner map; key of type Option<String>
///  │   │   ├─ file.ext  // inner map; value is Vec of supported files
///  │   │   └─ file.ext
///  │   └─ device_dir
///  │       └─ ...
///  └─ [assorted]
///      └─ single.file
/// ```
struct TargetDateDeviceTree {
    dir_tree: BTreeMap<String, DeviceTree>,
    unknown_extensions: HashSet<String>
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
                                format!("{} | {}", device_dir, file.file_path.display().to_string()))
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
            unknown_extensions: HashSet::new()
        }
    }

    /// Iterate all files in this this map and move all files which are in a directory with
    /// less than args.min_files_per_dir into a new separate directory (see [Args::oneoffs_dir_name])
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
        oneoffs_tree.file_tree.insert(DirEntryType::Files, oneoff_files);
        devices_tree.insert(args.oneoffs_dir_name.clone(), oneoffs_tree);

        self.dir_tree = devices_tree;

        self
    }

    /// Find the maximum length of the path string that may be present in the output
    /// This can only be computed after the tree has been filled with devices and files
    /// because of the requirement to only create device subdirs if there are at least 2 devices
    /// The resulting value covers two cases:
    /// - there's at least one date dir with >1 device subdirs -> target path length will be formed of `date/device_name`
    /// - there's no date dir with >1 devices -> target path will just include `date`
    /// Note: this must be called AFTER [Self::isolate_single_images()] so that the length of
    /// the oneoffs directory can be taken into account, if present
    fn compute_max_path_len(&mut self, args: &Args) -> usize {
        let max_date_dir_path_len = &self.dir_tree.iter()
            // filter only date dirs with at least 2 devices
            .filter(|(_, device_tree)| device_tree.file_tree.keys().clone().len() > 1 )
            // now search all devices for the max path len
            .map(|(_, device_tree)| device_tree.max_dir_path_len)
            .max();

        // We also need to account for the presence of a a oneoff directory. This is computed separately
        // and would not have been considered when setting `max_dir_path_len` during the initial iteration
        // If present, we compare its length now to the previous max. If not, assume 0 so we can ignore it
        let has_oneoffs_dir = &self.dir_tree.contains_key(args.oneoffs_dir_name.as_str());
        let oneoffs_dir_len = if *has_oneoffs_dir {
            get_string_char_count(args.oneoffs_dir_name.clone())
        } else {0};

        match *max_date_dir_path_len {
            Some(max_dir_path_len) =>
                max(max_dir_path_len, oneoffs_dir_len),
            None =>
                // default 10 for the length of date dirs, e.g. 2016.12.29
                max(10, oneoffs_dir_len)
        }
    }
}

#[derive(Debug)]
pub enum FileType {
    Unknown(String),
    Image,
    Video,
    Audio,
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
    aud_moved: i32,
    aud_copied: i32,
    aud_skipped: i32,
    unknown_skipped: i32,
    // source dirs which are skipped from reading
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
            aud_moved: 0,
            aud_copied: 0,
            aud_skipped: 0,
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
    fn inc_img_moved(&mut self) { self.img_moved += 1 }
    fn inc_img_copied(&mut self) { self.img_copied += 1 }
    fn inc_img_skipped(&mut self) { self.img_skipped += 1 }
    fn inc_vid_moved(&mut self) { self.vid_moved += 1 }
    fn inc_vid_copied(&mut self) { self.vid_copied += 1 }
    fn inc_vid_skipped(&mut self) { self.vid_skipped += 1 }
    fn inc_aud_moved(&mut self) { self.aud_moved += 1 }
    fn inc_aud_copied(&mut self) { self.aud_copied += 1 }
    fn inc_aud_skipped(&mut self) { self.aud_skipped += 1 }
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

    pub fn inc_copied_by_type(&mut self, file: &SupportedFile) {
        match file.file_type {
            FileType::Image   => self.inc_img_copied(),
            FileType::Video   => self.inc_vid_copied(),
            FileType::Audio   => self.inc_aud_copied(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown(_) => ()
        }
    }

    pub fn inc_moved_by_type(&mut self, file: &SupportedFile) {
        match file.file_type {
            FileType::Image   => self.inc_img_moved(),
            FileType::Video   => self.inc_vid_moved(),
            FileType::Audio   => self.inc_aud_moved(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown(_) => ()
        }
    }

    pub fn inc_skipped_by_type(&mut self, file: &SupportedFile) {
        match file.file_type {
            FileType::Image   => self.inc_img_skipped(),
            FileType::Video   => self.inc_vid_skipped(),
            FileType::Audio   => self.inc_aud_skipped(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown(_) => ()
        }
    }

    pub fn padded_color_if_non_zero(err_stat: i32, level: OutputColor, padding_width: usize) -> String {

        let padded_int = LeftPadding::space(err_stat.to_string(), padding_width);

        if err_stat > 0 {
            match level {
                OutputColor::Error =>
                    ColoredString::red(padded_int.as_str()),
                OutputColor::Warning =>
                    ColoredString::orange(padded_int.as_str()),
                Neutral =>
                    ColoredString::bold_white(padded_int.as_str()),
                OutputColor::Good =>
                    ColoredString::green(padded_int.as_str()),
            }
        } else {
            String::from(padded_int.to_string())
        }
    }

    pub fn color_if_non_zero(err_stat: i32, level: OutputColor) -> String {
        if err_stat > 0 {
            match level {
                OutputColor::Error =>
                    ColoredString::red(err_stat.to_string().as_str()),
                OutputColor::Warning =>
                    ColoredString::orange(err_stat.to_string().as_str()),
                Neutral =>
                    ColoredString::bold_white(err_stat.to_string().as_str()),
                OutputColor::Good =>
                    ColoredString::green(err_stat.to_string().as_str()),
            }
        } else {
            String::from(err_stat.to_string())
        }
    }

    pub fn print_stats(&self, args: &Args) {

        // add some empty space for wider spacing
        let max_digits = get_integer_char_count(self.files_count_total) + 1;

        let write_general_stats = || { format!(
"──────────────────────────────────────────────
Total files:                  {total} ({size})
──────────────────────────────────────────────
Images moved|copied|skipped:  │{p_img_move} │{p_img_copy} │{p_img_skip} │
Videos moved|copied|skipped:  │{p_vid_move} │{p_vid_copy} │{p_vid_skip} │
Audios moved|copied|skipped:  │{p_aud_move} │{p_aud_copy} │{p_aud_skip} │
──────────────────────────────────────────────
Folders created:              {dir_create}
Folders ignored:              {dir_ignore}
Unknown files skipped:        {f_skip}
File delete errors:           {fd_err}
File create errors:           {fc_err}
Folder create errors:         {dc_err}
──────────────────────────────────────────────
Time fetching folders:        {tfetch_dir} sec
Time parsing files:           {tparse_file} sec
Time writing files:           {twrite_file} sec
──────────────────────────────────────────────
Total time taken:             {t_total} sec
──────────────────────────────────────────────",
            total=FileStats::color_if_non_zero(self.files_count_total, Neutral),
            size=ColoredString::bold_white(get_file_size_string(self.file_size_total).as_str()),

            p_img_move=FileStats::padded_color_if_non_zero(self.img_moved, Neutral, max_digits),
            p_img_copy=FileStats::padded_color_if_non_zero(self.img_copied, Neutral, max_digits),
            p_img_skip=FileStats::padded_color_if_non_zero(self.img_skipped, Warning, max_digits),

            p_vid_move=FileStats::padded_color_if_non_zero(self.vid_moved, Neutral, max_digits),
            p_vid_copy=FileStats::padded_color_if_non_zero(self.vid_copied, Neutral, max_digits),
            p_vid_skip=FileStats::padded_color_if_non_zero(self.vid_skipped, Warning, max_digits),

            p_aud_move=FileStats::padded_color_if_non_zero(self.aud_moved, Neutral, max_digits),
            p_aud_copy=FileStats::padded_color_if_non_zero(self.aud_copied, Neutral, max_digits),
            p_aud_skip=FileStats::padded_color_if_non_zero(self.aud_skipped, Warning, max_digits),

            dir_create=FileStats::color_if_non_zero(self.dirs_created, Neutral),
            dir_ignore=FileStats::color_if_non_zero(self.dirs_ignored, Warning),

            f_skip=FileStats::color_if_non_zero(self.unknown_skipped, Warning),

            fd_err=FileStats::color_if_non_zero(self.error_file_delete, Error),
            fc_err=FileStats::color_if_non_zero(self.error_file_create, Error),
            dc_err=FileStats::color_if_non_zero(self.error_dir_create, Error),

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
        )}; // end write_general_stats

        let dryrun_general_stats = || { format!(
"–––––––––––––––––––––––––––––––––––––––––––––––
Total files:               {total} ({size})
–––––––––––––––––––––––––––––––––––––––––––––––
Images to move|copy|skip:  │{p_img_move} │{p_img_copy} │{p_img_skip} │
Videos to move|copy|skip:  │{p_vid_move} │{p_vid_copy} │{p_vid_skip} │
Audios to move|copy|skip:  │{p_aud_move} │{p_aud_copy} │{p_aud_skip} │
-----------------------------------------------
Target folders to create:  {dir_create}
Source folders to skip:    {dir_ignore}
Unknown files to skip:     {f_skip}
File delete errors:        {fd_err}
File create errors:        n/a
Folder create errors:      n/a
-----------------------------------------------
Time fetching folders:     {tfetch_dir} sec
Time parsing files:        {tparse_file} sec
Time printing files:       {twrite_file} sec
–––––––––––––––––––––––––––––––––––––––––––––––
Total time taken:          {t_total} sec
–––––––––––––––––––––––––––––––––––––––––––––––",
            total=FileStats::color_if_non_zero(self.files_count_total, Neutral),
            size=ColoredString::bold_white(get_file_size_string(self.file_size_total).as_str()),

            p_img_move=FileStats::padded_color_if_non_zero(self.img_moved, Neutral, max_digits),
            p_img_copy=FileStats::padded_color_if_non_zero(self.img_copied, Neutral, max_digits),
            p_img_skip=FileStats::padded_color_if_non_zero(self.img_skipped, Warning, max_digits),

            p_vid_move=FileStats::padded_color_if_non_zero(self.vid_moved, Neutral, max_digits),
            p_vid_copy=FileStats::padded_color_if_non_zero(self.vid_copied, Neutral, max_digits),
            p_vid_skip=FileStats::padded_color_if_non_zero(self.vid_skipped, Warning, max_digits),

            p_aud_move=FileStats::padded_color_if_non_zero(self.aud_moved, Neutral, max_digits),
            p_aud_copy=FileStats::padded_color_if_non_zero(self.aud_copied, Neutral, max_digits),
            p_aud_skip=FileStats::padded_color_if_non_zero(self.aud_skipped, Warning, max_digits),

            dir_create=FileStats::color_if_non_zero(self.dirs_created, Neutral),
            dir_ignore=FileStats::color_if_non_zero(self.dirs_ignored, Warning),

            f_skip=FileStats::color_if_non_zero(self.unknown_skipped, Warning),

            fd_err=FileStats::color_if_non_zero(self.error_file_delete, Error),

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
        )}; // end dryrun_general_stats

        // Print dry run stats
        if args.dry_run {
            println!("{}", dryrun_general_stats());

        // Print actual stats and other errors encountered when writing files
        } else {
            println!("{}", write_general_stats());

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
}

/// Enum entires meant to represent the target directories
/// named after the device name. Derive ordering and
/// equality traits for more natural ordering when used
/// as keys in a BTreeMap (will show files after directories)
#[derive(Clone, Debug, PartialOrd, Ord, PartialEq, Eq)]
pub enum DirEntryType {
    Directory(String),
    Files
}

impl fmt::Display for DirEntryType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Directory(dir_name) => dir_name.clone(),
            Self::Files => "".to_owned()
        })
    }
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
    device_name: DirEntryType,
    // index of the vec holding the files in the original source dir of this file
    // this is used to detect duplicates across multiple source dirs when doing dry runs
    source_dir_index: usize
}

// TODO 5e: find better name
impl SupportedFile {
    pub fn parse_from(dir_entry: DirEntry, source_index: usize, args: &Args) -> SupportedFile {
        let _extension = get_extension(&dir_entry);
        let _file_type = get_file_type(&_extension, args);
        let _metadata = dir_entry.metadata().unwrap();

        let _exif_data = match _file_type {
            // It's much faster if we only try to read EXIF for image files
            FileType::Image => {
                // Use kamadak-rexif crate
                let exif = read_kamadak_exif_date_and_device(&dir_entry, args);
                // Use rexif crate
                // let exif = read_exif_date_and_device(&dir_entry, args);
                exif
            },
            _ =>
                ExifDateDevice::new()
        };

        // Read image date - prefer EXIF tags over system date
        let _image_date = {
            _exif_data.date_original
                .unwrap_or(_exif_data.date_time
                    .unwrap_or(get_system_modified_date(&_metadata)
                        .unwrap_or(DEFAULT_NO_DATE_STR.to_string()))) };

        // Replace EXIF camera model with a custom name, if one was defined in config
        let _camera_name: DirEntryType = match _exif_data.camera_model {
            Some(camera_model) => args.custom_device_names
                .get(camera_model.to_lowercase().as_str())
                .map_or(DirEntryType::Directory(camera_model),
                        |custom_camera_name|DirEntryType::Directory(custom_camera_name.clone())),
            None =>
                DirEntryType::Files
        };

        SupportedFile {
            file_name: dir_entry.file_name(),
            file_path: dir_entry.path(),
            file_type: _file_type,
            extension: _extension,
            date_str: _image_date,
            metadata: _metadata,
            device_name: _camera_name,
            source_dir_index: source_index
        }
    }

    pub fn is_dir(&self) -> bool {
        self.metadata.is_dir()
    }

    pub fn get_file_name_str(&self) -> String {
        String::from(self.file_name.to_str().unwrap())
    }

    /// Return a string representation of the source file or path.
    /// If there are multiple sources, return the full absolute path
    /// If there is a single source, return only the filename,
    /// since the full path will always be the same
    pub fn get_source_display_name_str(&self, args: &Args) -> String {
        if args.has_multiple_sources() {
            format!("{}", self.file_path.display().to_string())
        } else {
            format!("{}", self.file_name.to_str().unwrap())
        }
    }
}

fn main() -> Result<(), std::io::Error> {

    let mut args = Args::new_from_toml("imgsorter.toml")?;

    let mut stats = FileStats::new();

    if args.debug {
        dbg!(&args);
    }

    // Needs to be created after checking for recursive source dirs,
    // since we need to pass args.has_multiple_sources()
    // let mut padder = Padder::new(args.has_multiple_sources());
    let mut padder = Padder::new(args.has_multiple_sources());


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

    let total_source_files: usize = source_contents.iter()
        .map(|dir|dir.len())
        .sum();


    // Exit early if there are no source files
    if total_source_files < 1 {
        println!("{}", ColoredString::red("There are no source files, exiting."));
        return Ok(());
    }

    {
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

        println!("═══════════════════════════════════════════════════════════════════════════");
        // TODO This would only be relevant if we're saving any files or reading config
        // println!("Current working directory: {}", &args.cwd.display());
        println!("{}", source_dirs);
        println!("Target directory:   {}", &args.target_dir.display());
        println!("Files to be {} {}", copy_status, total_source_files);
        println!("═══════════════════════════════════════════════════════════════════════════");
        // TODO 1f: print all options for this run?
    }

    // Proceed only if silent is enabled or user confirms, otherwise exit
    if args.silent {
        println! ("> Silent mode is enabled. Proceeding without user confirmation.");
        if args.dry_run {
            println!("> This is a dry run. No folders will be created. No files will be copied or moved.");
        }
    } else {
        match ask_for_confirmation(&args) {
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

    println!("–––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––");
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

    // Print unknown extensions
    if !target_dir_tree.unknown_extensions.is_empty() {
        println!("Skipped files with these unknown extensions: {}",
                 target_dir_tree.unknown_extensions
                     .into_iter()
                     .filter(|s|!s.is_empty())
                     .collect::<Vec<String>>().join(", "));
        println!();
    }

    // Print final stats
    stats.print_stats(&args);

    Ok(())
}

/// For dry runs over multiple source dirs we want to show if there are duplicate files across all sources.
/// This method iterates over all filenames in the source dirs, extracting a set for each dir.
/// Then it iterates all sets, checking each one against all previous ones and keeps only
/// unique elements, ensuring duplicate filenames are progressively removed.
fn get_source_unique_files(
    source_dir_contents: &Vec<Vec<DirEntry>>,
    args: &Args
) -> Option<Vec<HashSet<OsString>>> {

    // TODO 6i: this method only takes filename into consideration, should also consider date / device
    //   maybe even use [TargetDateDeviceTree] for this instead of source vecs?

    // This method is only useful for dry runs, return early otherwise
    if !args.dry_run {
        return None
    }

    let sets_of_files = source_dir_contents.iter().map(|src_dir|
        src_dir.iter()
            .map(|src_entry| src_entry.file_name())
            .collect::<HashSet<_>>())
        .collect::<Vec<_>>();

    if args.debug { print_sets_with_index("source file sets before filtering for uniques", &sets_of_files); }

    let all_unique_files = sets_of_files
        .iter().enumerate()
        .map(|(curr_ix, _)|
            // keep_unique_across_sets(&sets_of_files, curr_ix))
            keep_unique_across_sets(&sets_of_files[0..=curr_ix]))
        .collect::<Vec<_>>();

    if args.debug { print_sets_with_index("source file sets after filtering for uniques", &all_unique_files); }

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
                    if args.verbose {
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

    // If verbose is not enabled, print a generic message to show it's working
    // Otherwise, we'll print a progress message for each source directory
    if !args.verbose {
        println!("Reading source files...")
    }

    for (source_ix, source_dir) in source_dir_contents.into_iter().enumerate() {

        let time_parsing_dir = Instant::now();

        let current_file_count = source_dir.len();

        let mut skipped_files: Vec<String> = Vec::new();

        if args.verbose {
            // This is the first part of the progres line for this directory
            // See also the next [print_progress] call which prints the time taken to this same line
            // e.g. `[3566/4239] Parsing 2 files from D:\Temp\source_path\... done (0.018 sec)`
            print_progress(format!("[{}/{}] Parsing {} files from '{}'... ",
                                   count_so_far,
                                   total_no_files,
                                   current_file_count,
                                   args.source_dir[source_ix].display()));
        }

        // Parse each file into its internal representation and add it to the target tree
        for entry in source_dir {

            let current_file: SupportedFile = SupportedFile::parse_from(entry, source_ix, args);

            // Build final target path for this file
            match &current_file.file_type {
                FileType::Image | FileType::Video | FileType::Audio => {
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
                    let _device_name_len = match &current_file.device_name {
                        DirEntryType::Directory(dir_name) =>
                            get_string_char_count(dir_name.clone()),
                        DirEntryType::Files =>
                            0
                    };
                    let _date_name_str = &current_file.date_str.chars().count();
                    // add +1 for each path separator character
                    let total_target_path_len = _date_name_str + 1 + _device_name_len;

                    padder.set_max_source_filename_from_str(current_file.file_name.clone().to_str().unwrap());
                    padder.set_max_source_path(get_string_char_count(current_file.file_path.display().to_string()));
                    devicetree_for_this_date.max_dir_path_len = max(devicetree_for_this_date.max_dir_path_len, total_target_path_len);

                    // Add file to dir tree
                    all_files_for_this_device.push(current_file);
                }

                FileType::Unknown(ext) => {
                    stats.inc_unknown_skipped();
                    new_dir_tree.unknown_extensions.insert(ext.to_lowercase());
                    skipped_files.push(current_file.get_file_name_str());
                }
            }
        }

        // Record progress
        count_so_far += current_file_count;

        if args.verbose {
            // This is the second part of the progres line for this directory
            // See also the previous [print_progress] call which prints the first part of this line
            // e.g. `[3566/4239] Parsing 2 files from D:\Temp\source_path\... done (0.018 sec)`
            print_progress(format!("done ({}.{} sec)",
                                   time_parsing_dir.elapsed().as_secs(),
                                   LeftPadding::zeroes3(time_parsing_dir.elapsed().subsec_millis())));
            println!();
            // Print files intented with two spaces
            let skipped = skipped_files.into_iter()
                .filter(|s|!s.is_empty())
                .collect::<Vec<String>>();

            if !skipped.is_empty() {
                println!("Skipped unknown files:\n {}", skipped.join("\n "));
            }
        }
    }

    // This is a consuming call for now, so needs reassignment
    // TODO 5n: it shouldn't be consuming
    new_dir_tree = new_dir_tree.isolate_single_images(args);

    // The max path length can only be computed after the tree has been filled with devices and files
    // because of the requirement to only create device subdirs if there are at least 2 devices
    padder.set_max_target_path(new_dir_tree.compute_max_path_len(args));

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
        // TODO 5h need to pre-calculate max-depth length
        // TODO 5h FILE_TREE_INDENT is not required when there's only one level (i.e. one single device throughout)
        padder.add_extra_source_chars_from_str(DIR_TREE_INDENT_MID);
        padder.add_extra_source_chars_from_str(DIR_TREE_ENTRY_LAST);

        // TODO 5i: Refactor operation statuses and calculate this programatically
        let status_width = 20;
        let header_separator = padder.format_dryrun_header_separator(status_width);
        println!();
        println!("{}", ColoredString::bold_white(header_separator.as_str()));
        println!("{}", ColoredString::bold_white(
            padder.format_dryrun_header(status_width).as_str()));
        println!("{}", ColoredString::bold_white(header_separator.as_str()));

    } else {
        println!();
        let start_status = format!("Starting to {} files...", { if args.copy_not_move {"copy"} else {"move"}} );
        println!("{}", ColoredString::bold_white(start_status.as_str()));
        println!();

        let status_width = 20;
        let header_separator = padder.format_write_header_separator(status_width);
        println!("{}", ColoredString::bold_white(header_separator.as_str()));
        println!("{}", ColoredString::bold_white(
            padder.format_write_header(status_width).as_str()));
        println!("{}", ColoredString::bold_white(header_separator.as_str()));
    }


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

            let date_dir_name_with_device_status = {
                format!(
                    "[{dirname}] ({devicecount:?} {devicestr}, {filecount:?} {filestr}, {filesize}) ",
                    dirname = date_dir_name.clone(),
                    devicecount = device_count_for_date,
                    devicestr = _device_count_str,
                    filecount = file_count_for_date,
                    filestr = _file_count_str,
                    filesize = get_file_size_string(file_size_for_date))
            };

            // Check restrictions - if target exists
            let target_dir_exists = dry_run_check_target_dir_exists(&date_destination_path, stats);

            // Print everything together
            println!("{}",
                ColoredString::bold_white(
                format!("{dir_devices} {dir_status}",
                        dir_devices=padder.format_dryrun_date_dir(date_dir_name_with_device_status, args),
                        dir_status=target_dir_exists)
                    .as_str())
            );
        }


        /*****************************************************************************/
        /* ---            Iterate each device directory to be created            --- */
        /*****************************************************************************/

        // Count dirs to know which symbols to use for the dir tree
        // i.e. last entry is prefixed by └ and the rest by ├
        let dir_count_total = devices_files_and_paths.file_tree.len();
        let mut curr_dir_ix = 0 as usize;

        for (
            device_name_opt,
            files_and_paths_vec
        ) in &devices_files_and_paths.file_tree {

            curr_dir_ix += 1;
            let is_last_dir = curr_dir_ix == dir_count_total;

            // Maximum directory depth inside a date dir, starting from 0
            // Date Dir > 0. Device Dir > 1. File
            // Date Dir > 0. File
            let mut indent_level: usize = 0;

            // This condition helps prevent creating a redundant device subdir if
            // there's only a single Some("device") device (without any "None" device files)
            // Before                 After
            // ------                 -----
            // [date_dir]             [date_dir]
            //  └─ [device_dir]        │
            //      ├─ file01.ext      ├─ file01.ext
            //      └─ file02.ext      └─ file02.ext
            let has_at_least_one_distinct_device = {
                let _is_dir = device_name_opt.clone() != DirEntryType::Files;
                device_count_for_date > 1 && _is_dir
            };

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

            // If there's more than one DirEntryType, attach device dir to destination path
            let device_destination_path = if do_create_device_subdirs {
                // This is safe, since we've already checked the device is a Directory
                let device_dir_name = device_name_opt.to_string();

                // Attach device name as a new subdirectory to the current target path
                let device_path = date_destination_path.join(device_dir_name.clone()); // we only need clone here to be able to print it out later

                // Print device dir name
                if is_dry_run {

                    // Increase indent for subsequent files
                    indent_level += 1;

                    // Add tree indents and padding to dir name
                    let indented_device_dir_name = padder.format_dryrun_device_dir(
                        device_dir_name,
                        is_last_dir,
                        // if it's last dir, it's also the last element of type dir
                        is_last_dir,
                        args);

                    // Check restrictions - if target exists
                    let target_dir_status_check = dry_run_check_target_dir_exists(&device_path, stats);

                    // Print everything together
                    println!("{} {}", indented_device_dir_name, target_dir_status_check);
                }

                device_path

            // otherwise ignore device and just use the date dir
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

            // Count files to know which symbols to use for the dir tree
            // i.e. last entry is prefixed by └ and the rest by ├
            let file_count_total = files_and_paths_vec.len();

            for (file_index, file) in files_and_paths_vec.iter().enumerate() {

                let is_last_dir = curr_dir_ix == dir_count_total;
                let is_last_element = file_index == file_count_total - 1;

                // Attach filename to the directory path
                let mut file_destination_path = device_destination_path.clone()
                    .join(&file.file_name);

                let (padded_filename,
                    op_separator,
                    padded_path,
                    status_separator,
                    write_result,
                ) = {

                    // Output is different for dry-runs and copy/move operations, so print it separately
                    if is_dry_run {

                        // Prepare padded strings for output
                        let indented_target_filename = indent_string(
                            indent_level,
                            file.get_file_name_str(),
                            is_last_dir,
                            is_last_element);
                        let file_separator = padder.format_dryrun_file_separator(indented_target_filename.clone(), args);

                        let source_path = file.get_source_display_name_str(args);
                        let status_separator = padder.format_dryrun_status_separator_dotted(source_path.clone(), args);

                        // Check restrictions - file exists or is read-only
                        let file_restrictions = dry_run_check_file_restrictions(
                            &file,
                            &file_destination_path,
                            &source_unique_files,
                            args,
                            stats);

                        // Return everything to be printed
                        (indented_target_filename, file_separator, source_path, status_separator, file_restrictions)

                    } else {

                        // Prepare padded strings for output
                        let source_path = file.get_source_display_name_str(args);
                        let padded_separator = padder.format_write_file_separator(source_path.clone());
                        let stripped_target_path = file_destination_path.strip_prefix(&args.target_dir).unwrap().display().to_string();
                        let status_separator = padder.format_write_status_separator_dotted(stripped_target_path.clone());

                        // Copy/move file
                        let file_write_status = copy_file_if_not_exists(
                            &file,
                            &mut file_destination_path,
                            &args, &mut stats);

                        // Return everything to be printed
                        (source_path, padded_separator, stripped_target_path, status_separator, file_write_status)
                    }
                };

                // Print operation status - each separator is responsible for adding its own spaces where necessary
                println!("{left_side_file}{op_separator}{right_side_file}{status_separator}{status}",
                         left_side_file=padded_filename,
                         op_separator=op_separator,
                         right_side_file=padded_path,
                         status_separator=status_separator,
                         status=write_result);
            } // end loop files
        } // end loop device dirs

        // leave some empty space before the next date dir
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
fn dry_run_check_target_dir_exists(path: &PathBuf, stats: &mut FileStats) -> String {
    if path.exists() {
        // don't increase stats.inc_dirs_ignored() since it's not equivalent
        // a source directory which is skipped from reading
        String::from("[target folder exists, will not create]")
    } else {
        stats.inc_dirs_created();
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
    args: &Args,
    stats: &mut FileStats
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
            stats.inc_skipped_by_type(source_file);
            ColoredString::orange("duplicate source file, will be skipped")

        } else if target_path.exists() {
            stats.inc_skipped_by_type(source_file);
            ColoredString::orange("target file exists, will be skipped")

        } else if args.copy_not_move {
            stats.inc_copied_by_type(source_file);
            ColoredString::green("file will be copied")

        } else {

            // Check if the source file can be deleted after copy

            match source_file.file_path.metadata() {
                Ok(metadata) => {
                    let is_read_only = metadata.permissions().readonly();

                    if !args.copy_not_move && is_read_only {
                        stats.inc_error_file_delete();
                        stats.inc_copied_by_type(source_file);
                        ColoredString::red("source is read only, file will be copied")
                    } else {
                        stats.inc_moved_by_type(source_file);
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

fn ask_for_confirmation(args: &Args) -> ConfirmationType {
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
                if args.debug {
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

fn copy_file_if_not_exists(
    file: &SupportedFile,
    destination_path: &mut PathBuf,
    args: &Args,
    stats: &mut FileStats
) -> String {

    if destination_path.exists() {
        if args.debug {
            println!("> target file exists: {}",
                     &destination_path.strip_prefix(&args.target_dir).unwrap().display());
        }

        stats.inc_skipped_by_type(file);

        ColoredString::orange("already exists")

    } else {

        let copy_result = fs::copy(&file.file_path, &destination_path);

        match copy_result {

            // File creation was successful
            Ok(_) => {

                // If this is a MOVE, delete the source file after a successful copy and append status
                let (_delete_failed_opt, delete_result_str) = if !args.copy_not_move {

                    let delete_result = fs::remove_file(&file.file_path);

                    match delete_result {
                        Ok(_) =>
                            (Some(false), String::from(" (source file removed)")),
                        Err(e) => {
                            if args.verbose { eprintln!("File delete error: {:?}: ERROR {:?}", &file.file_path, e) };
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
                if args.copy_not_move || _delete_failed_opt.unwrap_or(false) {
                    stats.inc_copied_by_type(file);
                } else {
                    stats.inc_moved_by_type(file);
                }

                format!("{}{}",
                        ColoredString::green("ok"),
                        delete_result_str)
            },

            // Could not create target file, log error and don't even attempt to delete source
            Err(err) => {
                eprintln!("File copy error: {:?}: ERROR {:?}", &file.file_path, err);
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
        println!();
        println!("{}",
                 ColoredString::orange(
                     format!("[Folder {} already exists]",
                             target_subdir.strip_prefix(&args.target_dir).unwrap().display()).as_str()));
    } else {

        match fs::create_dir_all(target_subdir) {
            Ok(_) => {
                stats.inc_dirs_created();
                println!();
                println!("{}",
                         ColoredString::bold_white(
                             format!("[Created folder {}]",
                            target_subdir.strip_prefix(&args.target_dir).unwrap().display()).as_str()));
            },
            Err(e) => {
                // TODO 2f: handle dir creation fail?
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
fn get_file_type(extension_opt: &Option<String>, args: &Args) -> FileType {

    // Closure which checks if the file's extension is defined in the custom extensions
    let is_custom_extension = |ext: &String, file_type: &str| {
        args.custom_extensions.get(file_type).unwrap().contains(ext)
    };

    match extension_opt {
        Some(extension) => {
            match extension.to_lowercase().as_str() {
                // "Supported" extensions
                "jpg" | "jpeg" | "png" | "tiff" | "crw"| "nef" =>
                    FileType::Image,
                "mp4" | "mov" | "3gp" | "avi" =>
                    FileType::Video,
                "amr" | "ogg" =>
                    FileType::Audio,

                // User-configured extensions
                _ => {
                    if !args.custom_extensions.is_empty() {
                        if is_custom_extension(extension, IMAGE) {
                            FileType::Image
                        } else if is_custom_extension(extension, VIDEO) {
                            FileType::Video
                        } else if is_custom_extension(extension, AUDIO) {
                            FileType::Audio
                        } else {
                            FileType::Unknown(extension.clone())
                        }
                    } else {
                        FileType::Unknown(extension.clone())
                    }
                }
            }
        }
        None =>
            FileType::Unknown("".to_owned()),
    }
}