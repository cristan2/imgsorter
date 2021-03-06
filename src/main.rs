use std::cmp::max;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::fmt::Formatter;
use std::fs::{DirEntry, Metadata};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fmt, fs, io, thread};
use std::io::Read;
use std::ops::Add;
use itertools::Itertools;

use chrono::{DateTime, Utc};
use filesize::PathExt;

use imgsorter::config::*;
use imgsorter::exif::*;
use imgsorter::utils::*;
use OutputColor::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");


/// Convenience wrapper over a map holding all files for a given device
/// where the string representation of the optional device is the map key
#[derive(Debug)]
struct DeviceTree {
    file_tree: BTreeMap<DirEntryType, Vec<SupportedFile>>,
    max_dir_path_len: usize,
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
#[derive(Debug)]
struct TargetDateDeviceTree {
    dir_tree: BTreeMap<String, DeviceTree>,
    unknown_extensions: HashSet<String>,
}

/// Just output a simple list of filenames for now
impl fmt::Display for TargetDateDeviceTree {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let file_names: Vec<String> = self
            .dir_tree
            .iter()
            .map(|(date_dir, device_tree)| {
                let date_files = device_tree
                    .file_tree
                    .iter()
                    .flat_map(|(device_dir, files)| {
                        let device_files = files
                            .iter()
                            .map(|file| {
                                format!("{} -> {}", device_dir, file.file_path.display().to_string())
                            })
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
            unknown_extensions: HashSet::new(),
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
            return self;
        }

        let _has_single_device = |device_tree: &DeviceTree| device_tree.file_tree.keys().len() < 2;

        let _has_minimum_files = |device_tree: &DeviceTree| {
            let all_files_names = device_tree
                .file_tree
                .values()
                .flat_map(|files| files.iter().map(|f| f.file_name.clone()))
                .collect::<Vec<_>>();

            let all_files_unique: HashSet<&OsString> = HashSet::from_iter(all_files_names.iter());
            let all_files_count = all_files_unique.len();
            all_files_count < args.min_files_per_dir as usize
        };

        let has_oneoff_files = |device_tree: &DeviceTree| {
            _has_single_device(device_tree) && _has_minimum_files(device_tree)
        };

        // TODO 5h: this is inefficient, optimize to a single iteration and non-consuming method
        let mut devices_tree: BTreeMap<String, DeviceTree> = BTreeMap::new();
        let mut oneoffs_tree = DeviceTree::new();
        let mut oneoff_files: Vec<SupportedFile> = Vec::new();

        self.dir_tree
            .into_iter()
            .for_each(|(device_dir, device_tree)| {
                // Move single files from the current date dir to a separate dir,
                // which will be joined again later under a different key
                if has_oneoff_files(&device_tree) {
                    // TODO 6g handle max_len and possible file duplicates
                    device_tree
                        .file_tree
                        .into_iter()
                        .for_each(|(_, src_files)| oneoff_files.extend(src_files));

                // keep the existing date-device structure
                } else {
                    devices_tree.insert(device_dir, device_tree);
                }
            });

        if !oneoff_files.is_empty() {
            oneoffs_tree.file_tree.insert(DirEntryType::Files, oneoff_files);
            devices_tree.insert(args.oneoffs_dir_name.clone(), oneoffs_tree);
        }

        self.dir_tree = devices_tree;

        self
    }

    /// Find the maximum length of the path string that may be present in the output
    /// This can only be computed after the tree has been filled with devices and files
    /// because of the requirement to only create device subdirs if there are at least 2 devices
    ///   (unless always_create_device_subdirs is true, in which case >1 is ignored)
    /// The resulting value covers two cases:
    /// - there's at least one date dir with >1 device subdirs -> target path length will be formed of `date/device_name`
    /// - there's no date dir with >1 devices -> target path will just include `date`
    /// Note: this must be called AFTER [Self::isolate_single_images()] so that the length of
    /// the oneoffs directory can be taken into account, if present
    fn compute_max_path_len(&mut self, args: &Args) -> usize {

        let has_minimum_required_subdirs = |device_tree: &DeviceTree| {
            args.always_create_device_subdirs ||
                device_tree.file_tree.keys().clone().len() > 1
        };

        let max_date_dir_path_len = &self
            .dir_tree
            .iter()
            // filter only date dirs with at least 2 devices
            .filter(|(_, device_tree)| has_minimum_required_subdirs(device_tree))
            // now search all devices for the max path len
            .map(|(_, device_tree)| device_tree.max_dir_path_len)
            .max();

        // We also need to account for the presence of a a oneoff directory. This is computed separately
        // and would not have been considered when setting `max_dir_path_len` during the initial iteration
        // If present, we compare its length now to the previous max. If not, assume 0 so we can ignore it
        let has_oneoffs_dir = &self.dir_tree.contains_key(args.oneoffs_dir_name.as_str());
        let oneoffs_dir_len = if *has_oneoffs_dir {
            get_string_char_count(args.oneoffs_dir_name.clone())
        } else {
            0
        };

        match *max_date_dir_path_len {
            Some(max_dir_path_len) =>
                max(max_dir_path_len, oneoffs_dir_len),
            None =>
                // default 10 for the length of date dirs, e.g. 2016.12.29
                max(10, oneoffs_dir_len)
        }
    }

    // Merge two TargetDateDeviceTree
    fn extend(&mut self, other: TargetDateDeviceTree) {
        // append devices and files
        other.dir_tree
            .into_iter()
            .for_each(|(other_date_dir, other_device_tree)|{

                match self.dir_tree.get_mut(&other_date_dir) {
                    // if this date already exists, append devices to it
                    Some(devicetree_for_this_date) => {

                        other_device_tree.file_tree
                            .into_iter()
                            .for_each(| (other_device, other_files) |{

                                match devicetree_for_this_date.file_tree.get_mut(&other_device) {

                                    // if this device already exists, append files to it
                                    Some(all_files_for_this_device) => {
                                        all_files_for_this_device.extend(other_files);
                                    },

                                    // if this device is not found, just insert the other's entire file list
                                    None => {
                                        devicetree_for_this_date.file_tree.insert(other_device, other_files);
                                    }
                                }
                            });
                    }

                    // if this date is not found, just insert the other's entire device tree
                    None => {
                        self.dir_tree.insert(other_date_dir, other_device_tree);
                    }
                }
            });

        // append devices and files
        self.unknown_extensions.extend(other.unknown_extensions);
    }
}

#[derive(Debug)]
pub enum DirType {
    Date,
    Device,
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

/// Struct used to keep track of file statuses (i.e. future write restrictions)
/// when doing dry runs with output compacting enabled
#[derive(Debug)]
struct CompactCounter {
    compacting_threshold: usize,
    current_status_count: usize,
    current_status: String,
    skipped_status_count: usize,
}

impl CompactCounter {
    fn new(compacting_threshold: usize) -> CompactCounter {
        CompactCounter {
            compacting_threshold,
            current_status_count: 0,
            current_status: "".to_owned(),
            skipped_status_count: 0,
        }
    }

    fn reset_status(&mut self, new_status: String) {
        self.current_status_count = 0;
        self.current_status = new_status;
        self.skipped_status_count = 0;
    }

    fn inc_current_status(&mut self) {
        self.current_status_count += 1;
    }

    fn inc_skipped_status(&mut self) {
        self.skipped_status_count += 1;
    }

    fn has_reached_threshold(&self) -> bool {
        self.current_status_count >= self.compacting_threshold
    }

    fn has_skipped_statuses(&self) -> bool {
        self.skipped_status_count > 0
    }

    fn is_same_status(&self, new_status: &str) -> bool {
        self.current_status == *new_status
    }
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
    date_dirs_total: i32,
    date_dirs_created: i32,
    device_dirs_total: i32,
    device_dirs_created: i32,
    error_file_create: i32,
    error_file_delete: i32,
    error_date_dir_create: i32,
    error_device_dir_create: i32,
    time_fetch_files: Duration,
    time_fetch_dirs: Duration,
    time_parse_files: Duration,
    time_write_files: Duration,
    time_total: Duration,
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
            date_dirs_total: 0,
            date_dirs_created: 0,
            device_dirs_total: 0,
            device_dirs_created: 0,
            error_file_create: 0,
            error_file_delete: 0,
            error_date_dir_create: 0,
            error_device_dir_create: 0,
            time_fetch_files: Duration::new(0, 0),
            time_fetch_dirs: Duration::new(0, 0),
            time_parse_files: Duration::new(0, 0),
            time_write_files: Duration::new(0, 0),
            time_total: Duration::new(0, 0),
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
    fn inc_date_dirs_total(&mut self) { self.date_dirs_total += 1 }
    fn inc_date_dirs_created(&mut self) { self.date_dirs_created += 1 }
    fn inc_device_dirs_total(&mut self) { self.device_dirs_total += 1 }
    fn inc_device_dirs_created(&mut self) { self.device_dirs_created += 1 }
    pub fn inc_error_file_create(&mut self) { self.error_file_create += 1 }
    pub fn inc_error_file_delete(&mut self) { self.error_file_delete += 1 }
    pub fn inc_error_date_dir_create(&mut self) { self.error_date_dir_create += 1 }
    pub fn inc_error_device_dir_create(&mut self) { self.error_device_dir_create += 1 }
    pub fn set_time_fetch_files(&mut self, elapsed: Duration) { self.time_fetch_files = elapsed }
    pub fn set_time_fetch_dirs(&mut self, elapsed: Duration) { self.time_fetch_dirs = elapsed }
    pub fn set_time_parse_files(&mut self, elapsed: Duration) { self.time_parse_files = elapsed }
    pub fn set_time_write_files(&mut self, elapsed: Duration) { self.time_write_files = elapsed }
    pub fn set_time_total(&mut self, elapsed: Duration) { self.time_total = elapsed }

    pub fn inc_dir_total_by_type(&mut self, dir: &DirType) {
        match dir {
            DirType::Date => self.inc_date_dirs_total(),
            DirType::Device => self.inc_device_dirs_total(),
        }
    }

    pub fn inc_dir_created_by_type(&mut self, dir: &DirType) {
        match dir {
            DirType::Date => self.inc_date_dirs_created(),
            DirType::Device => self.inc_device_dirs_created(),
        }
    }

    pub fn inc_error_dir_create_by_type(&mut self, dir: &DirType) {
        match dir {
            DirType::Date => self.inc_error_date_dir_create(),
            DirType::Device => self.inc_error_device_dir_create(),
        }
    }

    pub fn inc_copied_by_type(&mut self, file: &SupportedFile) {
        match file.file_type {
            FileType::Image => self.inc_img_copied(),
            FileType::Video => self.inc_vid_copied(),
            FileType::Audio => self.inc_aud_copied(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown(_) => (),
        }
    }

    pub fn inc_moved_by_type(&mut self, file: &SupportedFile) {
        match file.file_type {
            FileType::Image => self.inc_img_moved(),
            FileType::Video => self.inc_vid_moved(),
            FileType::Audio => self.inc_aud_moved(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown(_) => (),
        }
    }

    pub fn inc_skipped_by_type(&mut self, file: &SupportedFile) {
        match file.file_type {
            FileType::Image => self.inc_img_skipped(),
            FileType::Video => self.inc_vid_skipped(),
            FileType::Audio => self.inc_aud_skipped(),
            // don't record any stats for this, shouldn't get one here anyway
            FileType::Unknown(_) => (),
        }
    }

    pub fn padded_color_if_non_zero(err_stat: i32, level: OutputColor, padding_width: usize) -> String {

        let padded_int = LeftPadding::space(err_stat.to_string(), padding_width);

        if err_stat > 0 {
            match level {
                OutputColor::Error   => ColoredString::red(padded_int.as_str()),
                OutputColor::Warning => ColoredString::orange(padded_int.as_str()),
                Neutral              => ColoredString::bold_white(padded_int.as_str()),
                OutputColor::Good    => ColoredString::green(padded_int.as_str()),
            }
        } else {
            padded_int
        }
    }

    pub fn color_if_non_zero(err_stat: i32, level: OutputColor) -> String {
        if err_stat > 0 {
            match level {
                OutputColor::Error   => ColoredString::red(err_stat.to_string().as_str()),
                OutputColor::Warning => ColoredString::orange(err_stat.to_string().as_str()),
                Neutral              => ColoredString::bold_white(err_stat.to_string().as_str()),
                OutputColor::Good    => ColoredString::green(err_stat.to_string().as_str()),
            }
        } else {
            err_stat.to_string()
        }
    }

    pub fn print_stats(&self, args: &Args) {
        // file count padding
        let f_max_digits = get_integer_char_count(self.files_count_total);
        // dir count padding; each should be half of the total file count width
        let d_max_digits = ((f_max_digits * 3) as f32 / 2_f32).ceil() as usize;

        let write_general_stats = || {
            format!(
"──────────────────────────────────────────────
Total files:                  {total} ({size})
──────────────────────────────────────────────
Images moved|copied|skipped:  │{p_img_move}│{p_img_copy}│{p_img_skip}│
Videos moved|copied|skipped:  │{p_vid_move}│{p_vid_copy}│{p_vid_skip}│
Audios moved|copied|skipped:  │{p_aud_move}│{p_aud_copy}│{p_aud_skip}│
──────────────────────────────────────────────
Date   folders created|total: │{date_d_create}│{date_d_total}│
Device folders created|total: │{devc_d_create}│{devc_d_total}│
Source folders ignored:       {dir_ignore}
Unknown files skipped:        {f_skip}
File delete errors:           {fd_err}
File create errors:           {fc_err}
Date folders create errors:   {date_c_err}
Device folders create errors: {devc_c_err}
──────────────────────────────────────────────
Time fetching folders:        {tfetch_dir} sec
Time fetching files:          {tfetch_file} sec
Time parsing files:           {tparse_file} sec
Time writing files:           {twrite_file} sec
──────────────────────────────────────────────
Total time taken:             {t_total} sec
──────────────────────────────────────────────",
            total=FileStats::color_if_non_zero(self.files_count_total, Neutral),
            size=ColoredString::bold_white(get_file_size_string(self.file_size_total).as_str()),

            p_img_move=FileStats::padded_color_if_non_zero(self.img_moved, Neutral, f_max_digits),
            p_img_copy=FileStats::padded_color_if_non_zero(self.img_copied, Neutral, f_max_digits),
            p_img_skip=FileStats::padded_color_if_non_zero(self.img_skipped, Warning, f_max_digits),

            p_vid_move=FileStats::padded_color_if_non_zero(self.vid_moved, Neutral, f_max_digits),
            p_vid_copy=FileStats::padded_color_if_non_zero(self.vid_copied, Neutral, f_max_digits),
            p_vid_skip=FileStats::padded_color_if_non_zero(self.vid_skipped, Warning, f_max_digits),

            p_aud_move=FileStats::padded_color_if_non_zero(self.aud_moved, Neutral, f_max_digits),
            p_aud_copy=FileStats::padded_color_if_non_zero(self.aud_copied, Neutral, f_max_digits),
            p_aud_skip=FileStats::padded_color_if_non_zero(self.aud_skipped, Warning, f_max_digits),

            date_d_create=FileStats::padded_color_if_non_zero(self.date_dirs_created, Neutral, d_max_digits),
            date_d_total=FileStats::padded_color_if_non_zero(self.date_dirs_total, Neutral, d_max_digits),

            devc_d_create=FileStats::padded_color_if_non_zero(self.device_dirs_created, Neutral, d_max_digits),
            devc_d_total=FileStats::padded_color_if_non_zero(self.device_dirs_total, Neutral, d_max_digits),

            dir_ignore=FileStats::color_if_non_zero(self.dirs_ignored, Warning),

            f_skip=FileStats::color_if_non_zero(self.unknown_skipped, Warning),

            fd_err=FileStats::color_if_non_zero(self.error_file_delete, Error),
            fc_err=FileStats::color_if_non_zero(self.error_file_create, Error),
            date_c_err=FileStats::color_if_non_zero(self.error_date_dir_create, Error),
            devc_c_err=FileStats::color_if_non_zero(self.error_device_dir_create, Error),

            tfetch_dir=ColoredString::bold_white(format!("{}:{}",
                self.time_fetch_dirs.as_secs(),
                LeftPadding::zeroes3(self.time_fetch_dirs.subsec_millis())).as_str()),
            tfetch_file=ColoredString::bold_white(format!("{}:{}",
                self.time_fetch_files.as_secs(),
                LeftPadding::zeroes3(self.time_fetch_files.subsec_millis())).as_str()),
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

        let dryrun_general_stats = || {
            format!(
"––––––––––––––––––––––––––––––––––––––––––––––––––––––
Total files:                    {total} ({size})
––––––––––––––––––––––––––––––––––––––––––––––––––––––
Images to move|copy|skip:       │{p_img_move}│{p_img_copy}│{p_img_skip}│
Videos to move|copy|skip:       │{p_vid_move}│{p_vid_copy}│{p_vid_skip}│
Audios to move|copy|skip:       │{p_aud_move}│{p_aud_copy}│{p_aud_skip}│
––––––––––––––––––––––––––––––––––––––––––––––––––––––
Date folders   to create|total: │{date_d_create}│{date_d_total}│
Device folders to create|total: │{devc_d_create}│{devc_d_total}│
––––––––––––––––––––––––––––––––––––––––––––––––––––––
Source folders to skip:         {dir_ignore}
Unknown files to skip:          {f_skip}
File delete errors:             n/a
File create errors:             n/a
Date folders create errors:     n/a
Device folders create errors:   n/a
-----------------------------------------------
Time fetching folders:          {tfetch_dir} sec
Time fetching files:            {tfetch_file} sec
Time parsing files:             {tparse_file} sec
Time printing files:            {twrite_file} sec
––––––––––––––––––––––––––––––––––––––––––––––––––––––
Total time taken:               {t_total} sec
––––––––––––––––––––––––––––––––––––––––––––––––––––––",
            total=FileStats::color_if_non_zero(self.files_count_total, Neutral),
            size=ColoredString::bold_white(get_file_size_string(self.file_size_total).as_str()),

            p_img_move=FileStats::padded_color_if_non_zero(self.img_moved, Neutral, f_max_digits),
            p_img_copy=FileStats::padded_color_if_non_zero(self.img_copied, Neutral, f_max_digits),
            p_img_skip=FileStats::padded_color_if_non_zero(self.img_skipped, Warning, f_max_digits),

            p_vid_move=FileStats::padded_color_if_non_zero(self.vid_moved, Neutral, f_max_digits),
            p_vid_copy=FileStats::padded_color_if_non_zero(self.vid_copied, Neutral, f_max_digits),
            p_vid_skip=FileStats::padded_color_if_non_zero(self.vid_skipped, Warning, f_max_digits),

            p_aud_move=FileStats::padded_color_if_non_zero(self.aud_moved, Neutral, f_max_digits),
            p_aud_copy=FileStats::padded_color_if_non_zero(self.aud_copied, Neutral, f_max_digits),
            p_aud_skip=FileStats::padded_color_if_non_zero(self.aud_skipped, Warning, f_max_digits),

            date_d_create=FileStats::padded_color_if_non_zero(self.date_dirs_created, Neutral, d_max_digits),
            date_d_total=FileStats::padded_color_if_non_zero(self.date_dirs_total, Neutral, d_max_digits),

            devc_d_create=FileStats::padded_color_if_non_zero(self.device_dirs_created, Neutral, d_max_digits),
            devc_d_total=FileStats::padded_color_if_non_zero(self.device_dirs_total, Neutral, d_max_digits),

            dir_ignore=FileStats::color_if_non_zero(self.dirs_ignored, Warning),

            f_skip=FileStats::color_if_non_zero(self.unknown_skipped, Warning),

            tfetch_dir=ColoredString::bold_white(format!("{}:{}",
                self.time_fetch_dirs.as_secs(),
                LeftPadding::zeroes3(self.time_fetch_dirs.subsec_millis())).as_str()),
            tfetch_file=ColoredString::bold_white(format!("{}:{}",
                self.time_fetch_files.as_secs(),
                LeftPadding::zeroes3(self.time_fetch_files.subsec_millis())).as_str()),
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

impl Default for FileStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Enum entries meant to represent the target directories
/// named after the device name. Derive ordering and
/// equality traits for more natural ordering when used
/// as keys in a BTreeMap (will show files after directories)
#[derive(Clone, Debug, PartialOrd, Ord, PartialEq, Eq)]
pub enum DirEntryType {
    Directory(String),
    Files,
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
}

// TODO 5e: find better name
impl SupportedFile {
    // TODO 10a - replace with parse_from_ref
    pub fn parse_from(dir_entry: DirEntry, args: &mut Args) -> SupportedFile {
        let extension = get_extension(&dir_entry);
        let file_type = get_file_type(&extension, args);
        let metadata = dir_entry.metadata().unwrap();

        let exif_data = match file_type {
            // It's much faster if we only try to read EXIF for image files
            FileType::Image => {
                // Use kamadak-rexif crate
                read_kamadak_exif_date_and_device(&dir_entry, args)
                // Use rexif crate
                // read_exif_date_and_device(&dir_entry, args)
            }
            _ => ExifDateDevice::new(),
        };

        // Replace EXIF camera model with a custom name, if one was defined in config
        let device_name: DirEntryType = match &exif_data.get_device_name(args.include_device_make) {
            Some(camera_model) =>
                args
                    .custom_device_names
                    .get(camera_model.to_lowercase().as_str())
                    .map_or(
                        {
                            args.non_custom_device_names.insert(camera_model.clone());
                            DirEntryType::Directory(camera_model.clone())
                        },
                        |custom_camera_name| DirEntryType::Directory(custom_camera_name.clone())
                    ),
            None if args.always_create_device_subdirs =>
                DirEntryType::Directory(DEFAULT_UNKNOWN_DEVICE_DIR_NAME.to_string()),
            None =>
                DirEntryType::Files,
        };

        // Read image date - prefer EXIF tags over system date
        let date_str = {
            exif_data.date
                .unwrap_or_else(|| get_system_modified_date(&metadata)
                    .unwrap_or_else(|| DEFAULT_NO_DATE_STR.to_string()))
        };

        SupportedFile {
            file_name: dir_entry.file_name(),
            file_path: dir_entry.path(),
            file_type,
            extension,
            date_str,
            metadata,
            device_name,
        }
    }

    // TODO 10a - almost-duplicate of parse_from, keep this one
    pub fn parse_from_ref(dir_entry: &DirEntry, args: &Args) -> (SupportedFile, HashSet<String>) {
        let extension = get_extension(&dir_entry);
        let file_type = get_file_type(&extension, args);
        let metadata = dir_entry.metadata().unwrap();

        let exif_data = match file_type {
            // It's much faster if we only try to read EXIF for image files
            FileType::Image => {
                // Use kamadak-rexif crate
                read_kamadak_exif_date_and_device(&dir_entry, args)
                // Use rexif crate
                // read_exif_date_and_device(&dir_entry, args)
            }
            _ => ExifDateDevice::new(),
        };

        let mut non_custom_device_names: HashSet<String> = HashSet::new();

        // Replace EXIF camera model with a custom name, if one was defined in config
        let device_name: DirEntryType = match &exif_data.get_device_name(args.include_device_make) {
            Some(camera_model) =>
                args
                    .custom_device_names
                    .get(camera_model.to_lowercase().as_str())
                    .map_or(
                        {
                            non_custom_device_names.insert(camera_model.clone());
                            DirEntryType::Directory(camera_model.clone())
                        },
                        |custom_camera_name| DirEntryType::Directory(custom_camera_name.clone())
                    ),
            None if args.always_create_device_subdirs =>
                DirEntryType::Directory(DEFAULT_UNKNOWN_DEVICE_DIR_NAME.to_string()),
            None =>
                DirEntryType::Files,
        };

        // Read image date - prefer EXIF tags over system date
        let date_str = {
            exif_data.date
                .unwrap_or_else(|| get_system_modified_date(&metadata)
                    .unwrap_or_else(|| DEFAULT_NO_DATE_STR.to_string()))
        };

        (
            SupportedFile {
            file_name: dir_entry.file_name(),
            file_path: dir_entry.path(),
            file_type,
            extension,
            date_str,
            metadata,
            device_name,
            },
            non_custom_device_names
        )
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
            self.file_path.display().to_string()
        } else {
            self.file_name.to_str().unwrap().to_string()
        }
    }
}

/// The main program body. This is an overview of the main flows:
/// * parse config file and set up args/run options
/// * read list of files in all source dirs
/// * ask for operation confirmation - dry run or write files
/// * parse source files and build a model of the destination dir structure (this is where the sorting occurs)
///   * if dry run, filter only source unique files
///   * parse files and sort them into the internal target dir model
///   * isolate one-off images
///   * calculate max file and path lengths for dry run output padding
/// * process files based on the destination dir model
///   * if dry run, only show target dir structure and potential copy status of each file
///   * if write, copy the files to the destination; if move is enabled, delete source files after copy
/// * print stats and exit
fn main() -> Result<(), std::io::Error> {

    println!("───────────────────────────────────────────────────────────────────────────");
    println!("                             IMGSORTER v{versn}                            ", versn = VERSION);
    println!("───────────────────────────────────────────────────────────────────────────");

    let mut args = Args::new_from_toml("imgsorter.toml")?;

    let mut stats = FileStats::new();

    if args.verbose { dbg!(&args); }

    // Needs to be created after checking for recursive source dirs,
    // since we need to pass args.has_multiple_sources()
    let mut padder = Padder::new(args.has_multiple_sources());

    /*****************************************************************************/
    /* ---                        Read source files                          --- */
    /*****************************************************************************/

    let time_fetching_files = Instant::now();

    // TODO 5g: instead of Vec<Vec<DirEntry>>, return a `SourceDirTree` struct
    //   which wraps the Vec's but contains additional metadata, such as no of files or total size
    // TODO 5p: make this multi-threaded
    // Read dir contents and filter out error results
    let source_files: BTreeMap<String, Vec<DirEntry>> = args
        .source_dirs
        .iter()
        .map(|src_dir_vec| {
            let parent_dir_name = src_dir_vec[0].display().to_string();
            let dir_contents = src_dir_vec
                .iter()
                .filter_map(|src_dir|
                    read_supported_files(src_dir, &mut stats, &args).ok())
                .flatten()
                .collect::<Vec<_>>();
            (parent_dir_name, dir_contents)
        })
        .collect::<BTreeMap<_, _>>();

    stats.set_time_fetch_files(time_fetching_files.elapsed());

    /*****************************************************************************/
    /* ---                 Print options before confirmation                 --- */
    /*****************************************************************************/

    // TODO 5l: use this in parse_source_dirs methods instead of recalculating it
    let source_files_count: usize = source_files.values().map(|d|d.len()).sum();

    // Exit early if there are no source files
    if source_files_count < 1 {
        println!("{}", ColoredString::red("There are no supported files in the current source(s), exiting."));
        return Ok(());
    }

    {
        let write_op = if args.copy_not_move {
            ColoredString::orange("copied:")
        } else {
            ColoredString::red("moved: ")
        };

        // Build the string used for printing source directory name(s) before confirmation
        let source_dirs_list: String = build_source_dirs_list_string(&args);

        println!("═══════════════════════════════════════════════════════════════════════════");
        println!("{}", source_dirs_list);
        println!("Target directory:   {}", &args.target_dir.display());
        println!("Files to be {} {}", write_op, source_files_count);
        println!("═══════════════════════════════════════════════════════════════════════════");
        // TODO 1f: print all options for this run?
    }

    // Proceed only if silent is enabled or user confirms, otherwise exit
    if args.silent {
        println!("> Silent mode is enabled. Proceeding without user confirmation.");
        if args.dry_run {
            println!("> This is a dry run. No folders will be created. No files will be copied or moved.");
        }
    } else {
        match ask_for_op_confirmation(&args) {
            ConfirmationType::Cancel => {
                println!("Cancelled by user, exiting.");
                return Ok(());
            }
            ConfirmationType::Error => {
                println!("Error confirming, exiting.");
                return Ok(());
            }
            ConfirmationType::DryRun => {
                println!("This is a dry run. No folders will be created. No files will be copied or moved.");
                args.dry_run = true;
            }
            ConfirmationType::Proceed =>
                args.dry_run = false
        }
    }

    let time_processing = Instant::now();

    println!("–––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––");
    println!();

    /*****************************************************************************/
    /* ---        Parse source files and copy/paste or dry run them          --- */
    /*****************************************************************************/

    // TODO 5j: prefilter for Images and Videos only
    // Iterate files, read modified date and create subdirs
    // Copy images and videos to subdirs based on modified date
    let time_parsing_files = Instant::now();

    let mut target_dir_tree = if args.max_threads == 1 {
        // TODO 10a: this should no longer be necessary
        parse_source_dirs(source_files, &mut args, &mut stats, &mut padder)
    } else {
        parse_source_dirs_threaded(source_files, &mut args, &mut stats, &mut padder)
    };

    stats.set_time_parse_files(time_parsing_files.elapsed());

    let time_writing_files = Instant::now();
    if !target_dir_tree.dir_tree.is_empty() {
        // Iterate files and either copy/move to subdirs as necessary
        // or do a dry run to simulate a copy/move pass
        process_target_dir_files(
            &mut target_dir_tree,
            &args,
            &mut stats,
            &mut padder,
        );
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
                     .map(|s|format!("'{}'", s))
                     .collect::<Vec<String>>().join(", "));
        println!();
    }

    // Print unknown extensions
    if !args.non_custom_device_names.is_empty() {
        println!("Device models with non-custom names: {}",
                 args.non_custom_device_names
                     .iter()
                     .map(|s|format!("'{}'", s))
                     .collect::<Vec<String>>().join(", "));
        println!();
    }

    // Print final stats
    stats.print_stats(&args);

    // Ask user input to prevent console window from closing before reading output
    if args.silent {
        println!("> Silent mode is enabled. Exiting without user confirmation.");
    } else {
        ask_for_exit_confirmation();
    }

    Ok(())
}

fn build_source_dirs_list_string(args: &Args) -> String {
    let source_dir_str = String::from("Source directory:   ");
    let source_dirs_str = String::from("Source directories: ");

    // TODO 5o: reimplement or at least extract this separately
    if args.has_multiple_sources() {
        // TODO 5o: need to re-calculate numbering padding and spacing for the second line
        let spacing_other_lines = " ".repeat(source_dirs_str.chars().count());

        // Show all source directories
        if args.verbose {
            let len_max_digits = get_integer_char_count(args.source_dirs_count as i32);
            args.source_dirs
                .iter()
                .enumerate()
                .map(|(outer_index, outer_src_path)| {
                    outer_src_path
                        .iter()
                        .enumerate()
                        .map(|(index, inner_src_path)| {
                            let _first_part = if outer_index == 0 && index == 0 { &source_dirs_str } else { &spacing_other_lines };
                            format!("{}{}-{}. {}",
                                    // print dir indexes starting from 1
                                    _first_part,
                                    outer_index + 1,
                                    LeftPadding::zeroes(index + 1, len_max_digits),
                                    &inner_src_path.display().to_string())
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                        .add("\n")
                })
                .collect::<Vec<_>>()
                .join("\n")
            // Show compact version of the source dirs - show only the outer (configured)
            // dirs and then just print a count of their inner dirs
        } else {
            let len_max_digits = get_integer_char_count(args.source_dirs.len() as i32);
            args.source_dirs
                .iter()
                .enumerate()
                .map(|(outer_index, outer_src)| {
                    let spacing_first_line = if outer_index == 0 { &source_dirs_str } else { &spacing_other_lines };
                    let first_line =
                        format!("{}{}. {}",
                                // print dir indexes starting from 1
                                spacing_first_line,
                                LeftPadding::zeroes(outer_index + 1, len_max_digits - 1),
                                &outer_src[0].display().to_string());

                    if outer_src.len() > 1 {
                        let second_line =
                            // subtract -1 since we're displaying the first item explicitly
                            format!("{}·- {} more subfolders",
                                    &spacing_other_lines,
                                    outer_src.len() - 1);

                        format!("{}\n{}", first_line, second_line)
                    } else {
                        first_line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    } else {
        format!("{}{}",
                source_dir_str,
                args.source_dirs[0]
                    .iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
        )
    }
}

/// Read contents of the provided dir but filter out subdirectories or files which failed to read
fn read_supported_files(
    source_dir: &Path,
    stats: &mut FileStats,
    args: &Args,
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
            .filter(|entry| {
                if entry.path().is_file() {
                    true
                } else {
                    if args.verbose {
                        println!(
                            "Recursive option is off, skipping subfolder {:?} in {:?}",
                            entry.file_name(), source_dir.file_name().unwrap());
                    }
                    stats.inc_dirs_ignored();
                    false
                }
            })
            .collect::<Vec<DirEntry>>()
    };

    Ok(filtered_entries)
}

/// Read directory and parse contents into supported data models
fn parse_source_dirs(
    source_dirs: BTreeMap<String, Vec<DirEntry>>,
    args: &mut Args,
    stats: &mut FileStats,
    padder: &mut Padder,
) -> TargetDateDeviceTree {
    let mut new_dir_tree: TargetDateDeviceTree = TargetDateDeviceTree::new();

    // TODO 5l: this should already be available from source_dir_contents metadata
    let total_no_files: usize = source_dirs.values().map(|vec| vec.len()).sum();

    stats.inc_files_total(total_no_files);

    let mut count_so_far = 0;

    // We'll print reading progress in two ways:
    // - if verbose, print a progress message in two parts for each source directory with time taken
    // - if not verbose, print a simple incrementing counter of individual files out of the total
    if !args.verbose {
        println!("Reading source files...")
    }

    for (source_dir_name, source_dir_contents) in source_dirs.into_iter() {
        let time_parsing_dir = Instant::now();

        let current_file_count = source_dir_contents.len();

        let mut skipped_files: Vec<String> = Vec::new();

        if args.verbose {
            // This is the first part of the progres line for this directory
            // See also the next [print_progress] call which prints the time taken to this same line
            // e.g. `[3566/4239] Parsing 2 files from D:\Temp\source_path\... done (0.018 sec)`
            print_progress(format!(
                "[{}/{}] ({}%) Parsing {} files from '{}'... ",
                count_so_far,
                total_no_files,
                simple_percentage(count_so_far, total_no_files),
                current_file_count,
                &source_dir_name,
            ));
        }

        // Parse each file into its internal representation and add it to the target tree
        for entry in source_dir_contents.into_iter() {
            // TODO 10a - replace with parse_from_ref
            let current_file: SupportedFile = SupportedFile::parse_from(entry, args);

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
                            .or_insert_with(DeviceTree::new)
                    };

                    // TODO 5i: replace these with single method in DeviceTree
                    let all_files_for_this_device = {
                        devicetree_for_this_date
                            .file_tree
                            .entry(file_device)
                            .or_insert_with(Vec::new)
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

                    padder.set_max_source_filename_from_str(
                        current_file.file_name.clone().to_str().unwrap());
                    padder.set_max_source_path(get_string_char_count(
                        current_file.file_path.display().to_string()));
                    devicetree_for_this_date.max_dir_path_len = max(
                        devicetree_for_this_date.max_dir_path_len,
                        total_target_path_len,
                    );

                    // Add file to dir tree
                    all_files_for_this_device.push(current_file);
                }

                FileType::Unknown(ext) => {
                    stats.inc_unknown_skipped();
                    new_dir_tree.unknown_extensions.insert(ext.to_lowercase());
                    skipped_files.push(current_file.get_file_name_str());
                }
            }

            if !args.verbose {
                count_so_far += 1;

                print_progress_overwrite(
                    format!("{}/{} ({}%)",
                            count_so_far, total_no_files, simple_percentage(count_so_far, total_no_files)).as_str());
            };
        }

        if args.verbose {

            // Record progress
            count_so_far += current_file_count;

            // This is the second part of the progres line for this directory
            // See also the previous [print_progress] call which prints the first part of this line
            // e.g. `[3566/4239] Parsing 2 files from D:\Temp\source_path\... done (0.018 sec)`
            print_progress(format!("done ({}.{} sec)",
                                   time_parsing_dir.elapsed().as_secs(),
                                   LeftPadding::zeroes3(time_parsing_dir.elapsed().subsec_millis())));
            println!();
            // Print files indented with two spaces
            let skipped = skipped_files
                .into_iter()
                .filter(|s| !s.is_empty())
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

    new_dir_tree
}

/// Read directory and parse contents into supported data models
fn parse_source_dirs_threaded(
    source_dirs: BTreeMap<String, Vec<DirEntry>>,
    args: &mut Args,
    stats: &mut FileStats,
    padder: &mut Padder,
) -> TargetDateDeviceTree {
    let mut new_dir_tree: TargetDateDeviceTree = TargetDateDeviceTree::new();

    // TODO 5l: this should already be available from source_dir_contents metadata
    let total_no_files: usize = source_dirs.values().map(|vec| vec.len()).sum();

    stats.inc_files_total(total_no_files);

    // let mut count_so_far = 0;

    let mut skipped_files: Vec<String> = Vec::new();

    // We'll print reading progress in two ways:
    // - if verbose, print a progress message in two parts for each source directory with time taken
    // - if not verbose, print a simple incrementing counter of individual files out of the total
    if !args.verbose {
        println!("Reading source files...")
    }

    let chunks_count = args.max_threads - 1;
    if args.verbose {
        println!("> using {} threads for {} files", chunks_count, total_no_files);
    }

    // TODO do we still need _source_dir_name?
    let source_files = source_dirs
        .into_iter()
        .flat_map(|(_source_dir_name, source_dir_contents)| { source_dir_contents })
        .collect::<Vec<_>>();

    let mut thread_handles = Vec::new();

    // split into owned chunks based on itertools and this answer:
    //   https://stackoverflow.com/questions/66446258/rust-chunks-method-with-owned-values
    let chunks: Vec<Vec<DirEntry>> = source_files.into_iter().chunks(chunks_count).into_iter().map(|chunk|chunk.collect()).collect();

    chunks
        .into_iter()
        .for_each(|source_entry_chunk| {
            let args_clone = args.clone();
            let handle= thread::spawn( move || {
                // TODO 10a: add progress indicator
                parse_dir_chunk(source_entry_chunk, &args_clone)
            });
            thread_handles.push(handle);
        });

    for handle in thread_handles {
        let chunk_result = handle.join().unwrap();

        new_dir_tree.extend(chunk_result.new_dir_tree);
        padder.set_max_source_filename(chunk_result.max_source_filename);
        padder.set_max_source_path(chunk_result.max_source_path);

        skipped_files.extend(chunk_result.skipped_files);
        stats.unknown_skipped += chunk_result.stats_unknown_skipped;
        args.non_custom_device_names.extend(chunk_result.non_custom_extensions);

        // TODO 10a: print skipped files?
    }

    // This is a consuming call for now, so needs reassignment
    // TODO 5n: it shouldn't be consuming
    new_dir_tree = new_dir_tree.isolate_single_images(args);

    // The max path length can only be computed after the tree has been filled with devices and files
    // because of the requirement to only create device subdirs if there are at least 2 devices
    padder.set_max_target_path(new_dir_tree.compute_max_path_len(args));

    new_dir_tree
}

fn parse_dir_chunk(source_entry_chunk: Vec<DirEntry>, args: &Args) -> ParseChunkResult {

    let mut skipped_files: Vec<String> = Vec::new();
    let mut new_dir_tree: TargetDateDeviceTree = TargetDateDeviceTree::new();
    let mut non_custom_extensions: HashSet<String> = HashSet::new();
    let mut stats_unknown_skipped: i32 = 0;
    let mut max_source_filename: usize = 0;
    let mut max_source_path: usize = 0;

    source_entry_chunk
        .into_iter()
        .for_each(|source_entry| {

            let (current_file, non_custom_ext) = SupportedFile::parse_from_ref(&source_entry, args);

            non_custom_extensions.extend(non_custom_ext);

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
                            .or_insert_with(DeviceTree::new)
                    };

                    // TODO 5i: replace these with single method in DeviceTree
                    let all_files_for_this_device = {
                        devicetree_for_this_date
                            .file_tree
                            .entry(file_device)
                            .or_insert_with(Vec::new)
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

                    let source_filename_len = get_string_char_count(
                        String::from(
                            current_file.file_name.clone().to_str().unwrap()));
                    let source_dir_path_len = get_string_char_count(
                        String::from(
                            current_file.file_path.display().to_string()));

                    max_source_filename = max(max_source_filename, source_filename_len);
                    max_source_path = max(max_source_path, source_dir_path_len);

                    devicetree_for_this_date.max_dir_path_len = max(
                        devicetree_for_this_date.max_dir_path_len,
                        total_target_path_len,
                    );

                    // Add file to dir tree
                    all_files_for_this_device.push(current_file);
                }

                FileType::Unknown(ext) => {
                    stats_unknown_skipped += 1;
                    new_dir_tree.unknown_extensions.insert(ext.to_lowercase());
                    skipped_files.push(current_file.get_file_name_str());
                }
            }

            // TODO 10a: redesign for multithreaded
            // if !args.verbose {
            //     count_so_far += 1;
            //
            //     print_progress_overwrite(
            //         format!("{}/{} ({}%)",
            //                 count_so_far, total_no_files, simple_percentage(count_so_far, total_no_files)).as_str());
            // };
        });

    ParseChunkResult {
        new_dir_tree,
        skipped_files,
        non_custom_extensions,
        stats_unknown_skipped,
        max_source_filename,
        max_source_path
    }
}

#[derive(Debug)]
struct ParseChunkResult {
    new_dir_tree: TargetDateDeviceTree,
    skipped_files: Vec<String>,
    non_custom_extensions: HashSet<String>,
    stats_unknown_skipped: i32,
    max_source_filename: usize,
    max_source_path: usize
}

/// Iterate the files according to the projected target structure and
/// either do a dry run and print resulting dir structure or
/// write the files to target as configured (copy or move)
fn process_target_dir_files(
    // The target tree representation of files to be copied/moved
    new_dir_tree: &mut TargetDateDeviceTree,
    args: &Args,
    mut stats: &mut FileStats,
    padder: &mut Padder,
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

    // This is useful only for dry runs, where we need to track unique files
    // as we iterate over them to be able to show the status of duplicates.
    // For write operations, this will remain unused and empty.
    let mut source_unique_files: HashSet<OsString> = HashSet::new();

    /*****************************************************************************/
    /* ---             Iterate each date directory to be created             --- */
    /*****************************************************************************/

    for (date_dir_name, devices_files_and_paths) in &new_dir_tree.dir_tree {
        let device_count_for_date = devices_files_and_paths.file_tree.keys().len();

        // Get a total sum of file counts and file size in a single iteration
        let (file_count_for_date, file_size_for_date) = devices_files_and_paths
            .file_tree
            .iter()
            .fold((0, 0), |(accum_count, accum_size), (_, files_and_paths)| {
                (
                    accum_count + files_and_paths.len(),
                    accum_size + get_files_size(files_and_paths),
                )});
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
            let target_dir_exists =
                dry_run_check_target_dir_exists(&date_destination_path, &DirType::Date, stats);

            // Print everything together
            println!("{}",
                ColoredString::bold_white(
                format!("{dir_devices} {dir_status}",
                        dir_devices=padder.format_dryrun_date_dir(date_dir_name_with_device_status, args),
                        dir_status=target_dir_exists)
                    .as_str())
            );
        } else {
            // Create date subdir
            create_subdir_if_required(&date_destination_path, &DirType::Date, args, &mut stats);
        }


        /*****************************************************************************/
        /* ---            Iterate each device directory to be created            --- */
        /*****************************************************************************/

        // Count dirs to know which symbols to use for the dir tree
        // i.e. last entry is prefixed by └ and the rest by ├
        let dir_count_total = devices_files_and_paths.file_tree.len();
        let mut curr_dir_ix = 0_usize;

        for (device_name_opt, files_and_paths_vec) in &devices_files_and_paths.file_tree {
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
            // via a messenger app and would end up in a "Sent" folder without EXIF info (computed device is None)
            // Before                 After
            // ------                 -----
            // [date_dir]             [date_dir]
            //  └─ [device_dir]        |
            //  │   └─ file01.ext      └─ file01.ext
            //  └─ file02.ext          └─ file02.ext
            // TODO 2g: add more logic to this case and maybe skip copying the file without EXIF info
            let has_double_file = device_count_for_date == 2 && file_count_for_date == 2;

            let do_create_device_subdirs = args.always_create_device_subdirs || has_at_least_one_distinct_device && !has_double_file;

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
                        args,
                    );

                    // Check restrictions - if target exists
                    let target_dir_status_check =
                        dry_run_check_target_dir_exists(&device_path, &DirType::Device, stats);

                    // Print everything together
                    println!("{} {}", indented_device_dir_name, target_dir_status_check);
                } else {
                    // Create device subdir
                    create_subdir_if_required(
                        &device_path, &DirType::Device, args, &mut stats);
                }

                device_path

            // otherwise ignore device and just use the date dir
            } else {
                date_destination_path.clone()
            };


            /*****************************************************************************/
            /* --- Iterate each file in a device directory and print or copy/move it --- */
            /*****************************************************************************/

            // Output is different for dry-runs and copy/move operations, so process them separately
            if is_dry_run {
                process_files_dry_run(files_and_paths_vec, device_destination_path,
                                      &mut source_unique_files, dir_count_total, curr_dir_ix, indent_level,
                                      args, &mut stats, padder)
            } else {
                process_files_write(files_and_paths_vec, device_destination_path,
                                    args, &mut stats, padder);
            };
        } // end loop device dirs

        // leave some empty space before the next date dir
        println!();

    } // end loop date dirs
}

/// Iterate all source files and print the estimated target directory structure.
/// Direction of arrows will be Right-to-Left to reflect focus on how the target
/// structure is created. Arrow lines are dashed to indicate nothing is written.
/// If compact mode is enabled, consecutive files with the same status above
/// a configured threshold will be omitted and replaced with a single "snipped" line.
/// Sample output:
/// ```
/// ---------------------------------------------------------------------------------
/// TARGET FILE                     SOURCE PATH                  OPERATION STATUS
/// ---------------------------------------------------------------------------------
/// [2019.01.28] (2 devices, 5 files, 3.34 MB) ................. [new folder will be created]
///  ├── [Canon 100D] .......................................... [new folder will be created]
///  │    ├── IMG-20190128.jpg <--- D:\Pics\IMG-20190128.jpg ... target file exists, will be skipped
///  │    ├── IMG-20190129.jpg <--- D:\Pics\IMG-20190129.jpg ... file will be copied
///  │    ·-- (snipped output for 1 files with same status)
///  └── IMG-20190127.jpg <-------- D:\Pics\IMG-20190127.jpg ... file will be copied
///  └── IMG-20190127.jpg <-------- D:\Pics - Copy\IMG-20190127.jpg ... duplicate source file, will be skipped
/// ```
fn process_files_dry_run(
    files_and_paths_vec: &[SupportedFile],
    device_destination_path: PathBuf,
    source_unique_files: &mut HashSet<OsString>,
    dir_count_total: usize,
    curr_dir_ix: usize,
    indent_level: usize,
    args: &Args,
    stats: &mut FileStats,
    padder: &mut Padder,
) {
    // Count files to know which symbols to use for the dir tree
    // i.e. last entry is prefixed by `└` and the rest by `├`
    let file_count_total = files_and_paths_vec.len();

    let mut compact_counter = CompactCounter::new(args.compacting_threshold);

    // Dry runs need also the index of each file to determine if it's the
    // last element in this dir to choose the appropriate dir tree symbol
    for (file_index, file) in files_and_paths_vec.iter().enumerate() {
        let is_last_dir = curr_dir_ix == dir_count_total;
        let is_first_element = file_index == 0;
        let is_last_element = file_index == file_count_total - 1;

        // Attach filename to the directory path
        let file_destination_path = device_destination_path.clone().join(&file.file_name);

        // Check restrictions - file exists or is read-only
        let file_restrictions = dry_run_check_file_restrictions(
            file,
            &file_destination_path,
            source_unique_files,
            args,
            stats,
        );

        let get_output_for_file = || {
            // Prepare padded strings for output
            let indented_target_filename = indent_string(
                indent_level,
                file.get_file_name_str(),
                is_last_dir,
                is_last_element,
            );

            let file_separator =
                padder.format_dryrun_file_separator(indented_target_filename.clone(), args);

            let source_path = file.get_source_display_name_str(args);
            let status_separator =
                padder.format_dryrun_status_separator_dotted(source_path.clone(), args);

            process_files_format_status(
                indented_target_filename,
                file_separator,
                source_path,
                status_separator,
                &file_restrictions,
            )
        };

        let get_snipped_output = |_compact_counter: &CompactCounter| {
            padder.format_dryrun_snipped_output(
                _compact_counter.skipped_status_count,
                indent_level,
                is_last_dir,
            )
        };

        // Output compacting is not enabled, print all file statuses directly
        // Ignore compacting when debug mode is enabled
        if !args.is_compacting_enabled() || args.verbose {
            let output = get_output_for_file();
            println!("{}", output);
        }

        // Output compacting is enabled, so print only the first few consecutive
        // files with the same status as configured under `args.compacting_threshold`
        else {
            // First iteration - nothing special to do, just initialize
            // all counters to 0 and move on to the next file
            if is_first_element {
                compact_counter.reset_status(file_restrictions.clone());
                compact_counter.inc_current_status();
                let output = get_output_for_file();
                println!("{}", output);
            }

            // Next iterations with the same status as before - print line
            // only if we haven't reached `args.compacting_threshold`,
            // otherwise don't print anything, just increment the skip count
            else if compact_counter.is_same_status(&file_restrictions) {
                if !compact_counter.has_reached_threshold() {
                    compact_counter.inc_current_status();
                    let output = get_output_for_file();
                    println!("{}", output);
                } else {
                    compact_counter.inc_skipped_status();
                }
            }

            // Next iterations, status has just changed, print skipped status for previous files
            // then reset all counters and continue with the current file
            else {
                if compact_counter.has_skipped_statuses() {
                    let output = get_snipped_output(&compact_counter);
                    println!("{}", output);
                }

                compact_counter.reset_status(file_restrictions.clone());
                compact_counter.inc_current_status();
                let output = get_output_for_file();
                println!("{}", output);
            }

            // After the last file, print any remaining skipped statuses before finishing
            if is_last_element && compact_counter.has_skipped_statuses() {
                let output = get_snipped_output(&compact_counter);
                println!("{}", output);
            }
        } // end else args.is_compacting_enabled
    } // end loop files
}

/// Iterate all source files and write them to target, printing the operation status.
/// Direction of arrows will be Left-to-Right to reflect the focus on the "write" operation.
/// Arrow lines are continuous to indicate the files are written. There is no compact
/// mode for this operation, since we want to show all available information.
/// Sample output:
/// ```
/// ─────────────────────────────────────────────────────────────────────────────────────────
/// SOURCE PATH                   TARGET FILE                                OPERATION STATUS
/// ─────────────────────────────────────────────────────────────────────────────────────────
/// [Created folder 2019.01.28]
/// D:\Pics\IMG-20190127.jpg ───> 2019.01.28\IMG-20190127.jpg .............. ok
/// D:\Pics\IMG-20190128.jpg ───> 2019.01.28\Canon 100D\IMG-20190128.jpg ... already exists
/// D:\Pics\IMG-20190129.jpg ───> 2019.01.28\Canon 100D\IMG-20190129.jpg ... ok
/// ```
fn process_files_write(
    files_and_paths_vec: &[SupportedFile],
    device_destination_path: PathBuf,
    args: &Args,
    mut stats: &mut FileStats,
    padder: &mut Padder,
) {
    for file in files_and_paths_vec.iter() {
        let mut file_destination_path = device_destination_path.clone().join(&file.file_name);

        // Prepare padded strings for output
        let source_path = file.get_source_display_name_str(args);
        let padded_separator = padder.format_write_file_separator(source_path.clone());
        let stripped_target_path = file_destination_path
            .strip_prefix(&args.target_dir)
            .unwrap()
            .display()
            .to_string();
        let status_separator =
            padder.format_write_status_separator_dotted(stripped_target_path.clone());

        // Copy/move file
        let file_write_status =
            copy_file_if_not_exists(file, &mut file_destination_path, args, &mut stats);

        // Print result
        let output = process_files_format_status(
            source_path,
            padded_separator,
            stripped_target_path,
            status_separator,
            &file_write_status,
        );

        println!("{}", output);
    }
}

fn process_files_format_status(
    left_side_file: String,
    op_separator: String,
    right_side_file: String,
    status_separator: String,
    op_status: &str,
) -> String {
    format!(
        "{}{}{}{}{}",
        left_side_file, op_separator, right_side_file, status_separator, op_status
    )
}

/// Read file metadata and return size in bytes
fn get_files_size(files: &[SupportedFile]) -> u64 {
    files
        .iter()
        .map(|file| {
            let f_path = &file.file_path;
            f_path.size_on_disk_fast(&file.metadata).ok().unwrap_or(0)
        })
        .sum()
}

/// Read a directory path and return a string signalling if the path exists
fn dry_run_check_target_dir_exists(
    path: &Path,
    dir_type: &DirType,
    stats: &mut FileStats,
) -> String {
    stats.inc_dir_total_by_type(dir_type);
    if path.exists() {
        // don't increase stats.inc_dirs_ignored() since it's not equivalent
        // a source directory which is skipped from reading
        String::from("[target folder exists, will not create]")
    } else {
        stats.inc_dir_created_by_type(dir_type);
        String::from("[new folder will be created]")
    }
}

/// Read a path and return a string signalling copy/move restrictions:
/// * in both cases, check if the source file exists - no copy will take place
/// * in both cases, check if the target file exists - file will be skipped
/// * in both cases, if there are multiple source dirs, check if the file is present more than once - skip all duplicates
/// * if this is a move, check if the source file is read-only and can't be moved (only copied)
fn dry_run_check_file_restrictions(
    source_file: &SupportedFile,
    target_path: &PathBuf,
    source_unique_files: &mut HashSet<OsString>,
    args: &Args,
    stats: &mut FileStats,
) -> String {

    // If this is the first time we've seen this file, store it so we can find duplicates later
    let mut is_source_unique = || {
        let path_string = target_path.clone().into_os_string();
        if source_unique_files.contains(&path_string) {
            false
        } else {
            source_unique_files.insert(path_string);
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
                }
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

fn ask_for_op_confirmation(args: &Args) -> ConfirmationType {
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
            Ok(input) => {
                if args.debug {
                    println!("User input: '{:?}'", input)
                }
            }
            Err(err) => {
                eprintln!("Error reading user input: {:?}", err);
                return ConfirmationType::Error;
            }
        }
        match user_input.trim().to_lowercase().as_str() {
            "n" | "no"  => return ConfirmationType::Cancel,
            "y" | "yes" => return ConfirmationType::Proceed,
            "d" | "dry" => return ConfirmationType::DryRun,
            _ => println!("...press one of 'y/yes', 'n/no' or 'd/dry', then Enter"),
        }
    }
}

fn ask_for_exit_confirmation() {
    println!("{}", ColoredString::magenta("Press Enter to exit"));
    io::stdin().read(&mut [0]).unwrap();
}

fn copy_file_if_not_exists(
    file: &SupportedFile,
    destination_path: &mut PathBuf,
    args: &Args,
    stats: &mut FileStats,
) -> String {
    if destination_path.exists() {
        if args.debug {
            println!(
                "> target file exists: {}",
                &destination_path
                    .strip_prefix(&args.target_dir)
                    .unwrap()
                    .display()
            );
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
                        Ok(_) => (Some(false), String::from(" (source file removed)")),
                        Err(e) => {
                            if args.verbose {
                                eprintln!("File delete error: {:?}: ERROR {:?}", &file.file_path, e)
                            };
                            stats.inc_error_file_delete();
                            (
                                Some(true),
                                ColoredString::red(
                                    format!(" (error removing source: {:?})", e.to_string())
                                        .as_str(),
                                ),
                            )
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

                format!("{}{}", ColoredString::green("ok"), delete_result_str)
            }

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
    target_subdir: &Path,
    dir_type: &DirType,
    args: &Args,
    stats: &mut FileStats
) {

    stats.inc_dir_total_by_type(dir_type);

    if target_subdir.exists() {
        // Don't need any stats here

        match dir_type {
            DirType::Device => {
                println!();
                println!("{}",
                         ColoredString::orange(
                             format!("[Folder {} already exists]",
                                     target_subdir.strip_prefix(&args.target_dir).unwrap().display()).as_str()));
            },
            // Don't print anything for date devices, it would be too many
            _ => {}
        }
    } else {
        match fs::create_dir_all(target_subdir) {
            Ok(_) => {
                stats.inc_dir_created_by_type(dir_type);
                println!();
                println!("{}",
                         ColoredString::bold_white(
                             format!("[Created folder {}]",
                                 if args.verbose {
                                     // This was just created successfully, so unwrap should be safe
                                     let canonical_path = target_subdir.canonicalize().unwrap();
                                     canonical_path.display().to_string()
                                 } else {
                                     target_subdir.strip_prefix(&args.target_dir).unwrap().display().to_string()
                                 }
                            ).as_str()));
            },
            Err(e) => {
                stats.inc_error_dir_create_by_type(dir_type);
                // TODO 2f: handle dir creation fail?
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
        .and_then(|os| os.to_str().map(String::from))
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
                // "Supported" image extensions
                "jpg" | "jpeg" | "png" | "tiff" | "heic"| "heif"| "webp" |
                    // Partially supported image extensions
                    "crw" | "nef" | "nrw" =>
                    FileType::Image,

                // "Supported" video extensions
                "avif" |
                    // Partially supported video extensions
                    "mp4" | "mov" | "3gp" | "avi" =>
                    FileType::Video,

                // Partially supported audio extensions
                "amr" | "ogg" | "m4a" =>
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
        None => FileType::Unknown("".to_owned()),
    }
}
