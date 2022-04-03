use std::collections::hash_set::Iter;
use std::collections::HashSet;
use std::cmp::max;
use std::iter::Cloned;
use std::ffi::OsString;
use std::io::Write;

pub struct ColoredString;

/// Provides static methods for formatting colored text based on ANSI codes
/// Taken from the following SO answers:
/// * [https://stackoverflow.com/questions/69981449/how-do-i-print-colored-text-to-the-terminal-in-rust]
/// * [https://stackoverflow.com/questions/287871/how-to-print-colored-text-to-the-terminal/287944#287944]
impl ColoredString {

    // Color codes:
    // * MAGENTA   = '\x1b[95m'
    // * BLUE      = '\x1b[94m'
    // * CYAN      = '\x1b[96m'
    // * GREEN     = '\x1b[92m'
    // * ORANGE    = '\x1b[93m'
    // * RED       = '\x1b[91m'
    // * NO_COLOR  = '\x1b[0m'
    // * BOLD      = '\x1b[1m'
    // * UNDERLINE = '\x1b[4m'

    pub fn magenta(s: &str) -> String { format!("\x1b[95m{}\x1b[0m", s) }
    pub fn blue(s: &str) -> String { format!("\x1b[94m{}\x1b[0m", s) }
    pub fn cyan(s: &str) -> String { format!("\x1b[96m{}\x1b[0m", s) }
    pub fn green(s: &str) -> String { format!("\x1b[92m{}\x1b[0m", s) }
    pub fn red(s: &str) -> String { format!("\x1b[91m{}\x1b[0m", s) }
    pub fn no_color(s: &str) -> String { format!("\x1b[0m{}\x1b[0m", s) }
    pub fn orange(s: &str) -> String { format!("\x1b[93m{}\x1b[0m", s) }
    pub fn bold_white(s: &str) -> String { format!("\x1b[1m{}\x1b[0m", s) }
    pub fn underline(s: &str) -> String { format!("\x1b[4m{}\x1b[0m", s) }

    pub fn warn_arrow() -> String { Self::orange(">") }
}

pub enum OutputColor {
    Error,
    Warning,
    Neutral,
    Good
}

/// Sample tree - CURRENT
/// ```
/// [2019.01.28] (2 devices, 3 files, 3.34 MB) ........................... [new folder will be created]
/// └── D:\Pics\IMG-20190128.jpg --------> 2019.01.28\IMG-20190128.jpg ... file will be copied
/// └── [Canon 100D] ..................................................... [new folder will be created]
/// |    └── D:\Pics\IMG-20190128.jpg ---> 2019.01.28\IMG-20190128.jpg ... file will be copied
/// |    └── D:\Pics\IMG-20190128.jpg ---> 2019.01.28\IMG-20190128.jpg ... file will be copied
///
/// Sample tree - AFTER
/// [2019.01.28] (2 devices, 3 files, 3.34 MB) ........................... [new folder will be created]
/// └── IMG-20190128.jpg <-------- D:\Pics\IMG-20190128.jpg ... file will be copied
/// └── [Canon 100D] ..................................................... [new folder will be created]
/// |    └── IMG-20190128.jpg <--- D:\Pics\IMG-20190128.jpg ... file will be copied
/// |    └── IMG-20190128.jpg <--- D:\Pics\IMG-20190128.jpg ... file will be copied
/// ```
pub struct Padder {
    // BASIC INFO

    /// Whether there's a single source directory or multiple
    /// This matters when outputting source paths - for single sources we'd only
    /// need to output the filname, since the full path will always be the same
    has_multiple_sources: bool,

    /// The maximum length of filename of all source files,
    /// without any path information, e.g. `IMG-20190128.jpg`
    pub source_base_file_max_len: usize,
    /// The maximum length of the absolute path length of all source files,
    /// including the file name, e.g. `D:\Pics\IMG-20190128.jpg`
    pub source_path_max_len: usize,
    /// This maximum length of the relative target path from the parent target dir
    /// This *does not* include the filename length, which can always be read
    ///   from [source_base_file_max_len] (and adding 1 for the separator char)
    /// So this will include either the "date\device name", or just the "date", e.g.:
    /// `2019.01.28\Canon 100` or just `2019.01.28`
    pub target_relative_path_max_len: usize,

    // Length of any additional glyphs or words which are added to the source file
    // when printed, such as dir tree indents or other separators
    pub extra_source_chars: usize,

    pub max_depth_level: usize,

    // int: source max_len - either filename or full path
    // int: target max_len - date\device\filename
    // int: operation status max len

    // ADDITIONAL SIGNS
    // int: max depth size -> calculate FILE_TREE_INDENT x max_depth_size +
    // string: operation_separator
    // string: status separator
}
pub struct RightPadding;
pub struct LeftPadding;

impl Padder {
    pub fn new(has_multiple_sources: bool) -> Padder {
        Padder{
            has_multiple_sources,
            source_base_file_max_len: 0,
            source_path_max_len: 0,
            target_relative_path_max_len: 0,
            extra_source_chars: 0,
            // TODO temporary hardcode to 1, should be set by code
            // maximum directory depth inside a date dir, starting from 0
            // Date Dir > 0. Device Dir > 1. File
            // Date Dir > 0. File
            max_depth_level: 1,
        }
    }

    // INTERNAL API
    // fn _total_source_len()
    // fn _total_target_len()
    // fn _total_max_len() = dir_tre_indents + target + op_sep + source + status_sep

    // EXTERNAL API
    // fn format_date_dir(dir_name+device_status) - pad to total max len
    // fn format_device_dir(dir_name) - indent_string(0, format!("[{}] ", dir_name))
    // // There are no sub-subdirs possible, so there will only ever be a single dir level under a date level
    // // fn format_dir(dir_name, level) - indent_string(level, format!("[{}] ", dir_name))
    // fn format_target_path(path, depth)
    // fn format_source_path(path)
    // ??? fn format_files(source_path_or_filename, target_path, target_depth, op_status)

    // fn format_header_separator() - pad to total max len + max len status
    // fn format_header_target() - pad to target max_len + additional signs
    // fn format_header_source() - pad to source max_len

    pub fn set_max_source_filename(&mut self, new_file_len: usize) {
        self.source_base_file_max_len = max(self.source_base_file_max_len, new_file_len)
    }

    pub fn set_max_source_path(&mut self, new_path_len: usize) {
        self.source_path_max_len = max(self.source_path_max_len, new_path_len)
    }

    pub fn set_max_target_path(&mut self, new_path_len: usize) {
        self.target_relative_path_max_len = max(self.target_relative_path_max_len, new_path_len)
    }

    pub fn add_extra_source_chars(&mut self, new_len: usize) {
        self.extra_source_chars += new_len
    }

    pub fn set_max_source_filename_from_str(&mut self, new_file_name: &str) {
        self.set_max_source_filename(get_string_char_count(String::from(new_file_name)));
    }

    pub fn set_max_source_path_from_str(&mut self, new_path: &str) {
        self.set_max_source_path(get_string_char_count(String::from(new_path)));
    }

    pub fn add_extra_source_chars_from_str(&mut self, extra: &str) {
        self.add_extra_source_chars(get_string_char_count(String::from(extra)));
    }

    /* --- Getter methods --- */

    fn get_source_len(&self) -> usize {
        if self.has_multiple_sources {
            self.source_path_max_len
        } else {
            self.source_base_file_max_len
        }
    }

    // ONLY FOR SOURCE-TO-TARGET OUTPUT
    /// Retrieves the total length of the source - either just the filename
    ///  or the full path, depending on whether we have multiple sources - plus
    /// any additional symbols, like tree indents
    // fn get_total_max_source_len(&self) -> usize {
    //     let base = self.get_source_len();
    //     base + self.extra_source_chars
    // }

    // ONLY FOR SOURCE-TO-TARGET OUTPUT
    /// Retrieves the total relative target path, including the filename
    // fn get_total_max_target_len(&self) -> usize {
    //     // add +1 for the length of the separator between path and filename
    //     self.target_relative_path_max_len + 1 + self.source_base_file_max_len
    // }

    // TODO separate method not be necessary if not doing other calculation
    fn get_total_max_source_len(&self) -> usize {
        self.get_source_len()
    }

    /// Used for dry runs. This calculates the max target length
    /// which is composed of any file tree symbols plus the base filename
    fn get_total_max_target_len(&self) -> usize {
        self.source_base_file_max_len + self.extra_source_chars
    }

    /// Used for write outputs. This calculates the max target path length
    /// which is composed of the relative target path plus the base filename
    fn get_max_target_path_len(&self) -> usize {
        // add +1 for the separator between the path and the filename
        self.target_relative_path_max_len + 1 + self.source_base_file_max_len
    }

    // TODO cache the result of these get functions, don't calculate it each time

    // ONLY FOR SOURCE-TO-TARGET OUTPUT
    // fn get_total_padding_len(&self) -> usize {
    //     self.get_total_max_source_len()
    //         + 1 // add +1 for the gap between a filename and its padding
    //         + SEPARATOR_DRY_RUN_LEFT_TO_RIGHT.chars().count()
    //         + self.get_total_max_target_len()
    //         + SEPARATOR_STATUS.chars().count()
    //         + 1 // add +1 for the gap between a path and its padding
    //         + 1 // add +1 for the gap between a path and the operation status
    // }

    // TODO split total_max_target into extra_symbols + max_target ?
    fn get_total_padding_len(&self) -> usize {
        self.get_total_max_target_len()
            + 1 // add +1 for the gap between the target filename and the operation separator
            + SEPARATOR_DRY_RUN_LEFT_TO_RIGHT.chars().count()
            + 1 // add +1 for the gap between the source file/path and its the status separator
            + self.get_total_max_source_len()
            + 1 // add +1 for the gap between a path and the operation status
            + SEPARATOR_STATUS.chars().count()
    }

    /// This separator should fill the space between the current filename and the
    /// maximum target filename length (including the dir tree symbols in both cases)
    /// The calculation is based assuming the target file is printed to the left of the separator
    fn get_dryrun_file_separator_padding_len(&self, indented_target_filename: String) -> usize {
        let indented_target_filename_length = get_string_char_count(indented_target_filename);
        self.get_total_max_target_len()
            - indented_target_filename_length
            + SEPARATOR_DRY_RUN_LEFT_TO_RIGHT.chars().count()
    }

    /// This separator should fill the space between the current filename and the maximum source filename length.
    /// The calculation is based assuming the source path is printed to the left of the separator
    fn get_write_file_separator_padding_len(&self, source_path: String) -> usize {
        let source_path_length = get_string_char_count(source_path);
        self.get_total_max_source_len()
            - source_path_length
            + SEPARATOR_COPY_MOVE.chars().count()
    }

    /// This separator should fill the space between the
    /// source path and the estimated result of the operation
    fn get_dryrun_status_separator_padding_len(&self, source_path: String) -> usize {
        let source_path_length = get_string_char_count(source_path);
        self.get_total_max_source_len()
            - source_path_length
            + SEPARATOR_STATUS.chars().count()
    }

    /// This separator should fill the space between the
    /// target path and the result of the operation
    fn get_write_status_separator_padding_len(&self, target_path: String) -> usize {
        let target_path_length = get_string_char_count(target_path);
        self.get_max_target_path_len()
            - target_path_length
            + SEPARATOR_STATUS.chars().count()
    }

    fn get_source_padding_len(&self, padded_target_filename_length: String) -> usize {
        let target_len = get_string_char_count(padded_target_filename_length);
        self.get_total_padding_len()
            - target_len
            - 1
            - SEPARATOR_DRY_RUN_LEFT_TO_RIGHT.chars().count()
            - SEPARATOR_DRY_RUN_LEFT_TO_RIGHT.chars().count()
    }

    /* --- Formatter methods --- */

    /// Adds dot padding to the maximum padding length for the date dir, e.g.:
    /// `[2019.01.28] (2 devices, 3 files, 3.34 MB) ..........................`
    pub fn format_date_dir(&self, date_dir_name_with_device_status: String) -> String {
        RightPadding::dot(
            date_dir_name_with_device_status,
            self.get_total_padding_len())
    }

    /// Adds dot padding to the maximum padding length for the device dir.
    /// The device dirs will always have a single dir tree symbol prefix,
    /// since we don't expect additional sublevels for the devices, e.g.:
    /// `└── [Canon 100D] ..............................`
    pub fn format_device_dir(&self, device_dir_name: String) -> String {
        let indented_device_dir_name: String = indent_string(
            // There are no indent levels for device dirs, just add
            0, format!("[{}] ", device_dir_name));

        RightPadding::dot(
            indented_device_dir_name,
            // safe to unwrap for dry runs
            self.get_total_padding_len())
    }

    // ONLY FOR SOURCE-TO-TARGET OUTPUT (used only for copy/move, not dry runs)
    pub fn format_target_file_dotted(&self, mut filename: String) -> String {
        // Add a space after the filename so there's a gap until the padding starts
        filename.push(' ');
        RightPadding::dot(
            format!("{}", filename),
            // add +1 for the space added to the filename
            self.get_total_max_target_len() + 1)
    }

    // ONLY FOR SOURCE-TO-TARGET OUTPUT
    // pub fn format_source_file_indented_dashed(&self, mut filename: String, indent_level: usize) -> String {
    //     // Add a space after the filename so there's a gap until the padding starts
    //     filename.push(' ');
    //     let indented_source_filename = indent_string(indent_level, filename);
    //     RightPadding::dash(
    //         format!("{}", indented_source_filename),
    //         // add +1 for the space added to the filename
    //         self.get_total_max_source_len() + 1)
    // }

    pub fn format_source_dotted(&self, mut filename: String) -> String {
        RightPadding::dot(
            // Add a space after the filename so there's a gap until the padding starts
            format!("{} ", filename),
            // add +1 for the space added to the filename
            self.get_total_max_source_len() + 1)
    }

    pub fn format_file_separator_dashed(&self, left_file: String) -> String {
        let padded_separator = RightPadding::dash(
            // Add a space to the left so there's a gap between the previous file and the separator
            format!(" {}", SEPARATOR_DRY_RUN_RIGHT_TO_LEFT),
            // add +1 for the space added before the separator
            self.get_dryrun_file_separator_padding_len(left_file) + 1);
        // Add a space to the right so there's a gap between the separator and the next file
        format!("{} ", padded_separator)
    }

    pub fn format_file_separator_emdashed(&self, left_file: String) -> String {
        let padded_separator = LeftPadding::em_dash(
            // Add a space to the left so there's a gap between the file and the separator
            format!("{} ", SEPARATOR_COPY_MOVE),
            // add +1 for the space added before the separator
            self.get_write_file_separator_padding_len(left_file) + 1);
        // Add a space to the right so there's a gap between the separator and the source file
        format!(" {}", padded_separator)
    }

    pub fn format_dryrun_status_separator_dotted(&self, left_file: String) -> String {
        let padded_separator = RightPadding::dot(
            // Add a space to the left so there's a gap between the target file and the separator
            format!(" {}", SEPARATOR_STATUS),
            // add +1 for the space added before the separator
            self.get_dryrun_status_separator_padding_len(left_file) + 1);
        // Add a space to the right so there's a gap between the separator and the source file
        format!("{} ", padded_separator)
    }

    pub fn format_write_status_separator_dotted(&self, left_file: String) -> String {
        let padded_separator = RightPadding::dot(
            // Add a space to the left so there's a gap between the target file and the separator
            format!(" {}", SEPARATOR_STATUS),
            // add +1 for the space added before the separator
            self.get_write_status_separator_padding_len(left_file) + 1);
        // Add a space to the right so there's a gap between the separator and the source file
        format!("{} ", padded_separator)
    }

    // ONLY FOR SOURCE-TO-TARGET OUTPUT
    // pub fn format_target_file_indented(&self, mut filename: String, indent_level: usize) -> String {
    //     // Add a space after the filename so there's a gap until the padding starts
    //     filename.push(' ');
    //     let indented_source_filename = indent_string(indent_level, filename);
    //     RightPadding::dash(
    //         format!("{}", indented_source_filename),
    //         // add +1 for the space added to the filename
    //         self.get_total_max_source_len() + 1)
    // }

    pub fn format_source_file_left_dashed(&self, mut source_filename: String, padded_target_filename_length: String) -> String {
        // Add a space before the filename so there's a gap until the padding starts
        source_filename.insert_str(0, " ");
        LeftPadding::dash(
            format!("{}", source_filename),
            // add +1 for the space added to the filename
            // self.get_source_padding_len())
        self.get_source_padding_len(padded_target_filename_length) + 1)
    }

    pub fn format_target_file_indented(&self, mut filename: String, indent_level: usize) -> String {
        // Add a space after the filename so there's a gap until the padding starts
        filename.push(' ');
        indent_string(indent_level, filename)
    }

    pub fn format_source_file_indented_em_dashed(&self, mut filename: String) -> String {
        // Add a space after the filename so there's a gap until the padding starts
        filename.push(' ');
        RightPadding::em_dash(
            filename,
            self.get_total_max_source_len()
                // add +1 for the space added to the filename
                + 1)
    }

}

impl RightPadding {
    // TODO 5g - have char as argument
    pub fn space(str: String, pad_width: usize) -> String {
        format!("{:<width$}", str, width=pad_width)
    }

    pub fn dot(str: String, pad_width: usize) -> String {
        format!("{:.<width$}", str, width=pad_width)
    }

    pub fn dash(str: String, pad_width: usize) -> String {
        format!("{:-<width$}", str, width=pad_width)
    }

    pub fn em_dash(str: String, pad_width: usize) -> String {
        format!("{:─<width$}", str, width=pad_width)
    }

    pub fn middle_dot(str: String, pad_width: usize) -> String {
        format!("{:·<width$}", str, width=pad_width)
    }
}

impl LeftPadding {
    pub fn zeroes3<T: Into<u64>>(no: T) -> String {
        format!("{:0>width$}", no.into(), width=3)
    }

    pub fn em_dash(str: String, pad_width: usize) -> String {
        format!("{:─>width$}", str, width=pad_width)
    }

    pub fn dash(str: String, pad_width: usize) -> String {
        format!("{:->width$}", str, width=pad_width)
    }
}

pub const SEPARATOR_STATUS: &'static str = "...";
pub const SEPARATOR_DRY_RUN_LEFT_TO_RIGHT: &'static str = "--->";
pub const SEPARATOR_DRY_RUN_RIGHT_TO_LEFT: &'static str = "<---";
pub const SEPARATOR_COPY_MOVE: &'static str = "───>";
pub const FILE_TREE_ENTRY: &'static str = " └── ";
pub const FILE_TREE_INDENT: &'static str = " |   ";

/// Adds dir tree symbols in front of the string based on the indent level.
/// If level > 0, string gets an equal number of [FILE_TREE_INDENT] prefixes.
/// All strings get a [FILE_TREE_ENTRY] prefix. For example:
/// ```
/// [2019.01.28]
/// └── IMG-20190128.jpg
/// └── [Canon 100D]
/// |    └── IMG-20190128.jpg
/// |    └── IMG-20190128.jpg
/// ```
pub fn indent_string(indent_level: usize, file_name: String) -> String {
    let indents = FILE_TREE_INDENT.repeat(indent_level);
    format!("{}{}{}", indents, FILE_TREE_ENTRY.to_string(), file_name)
}


/// For any given vec of sets of filenames, check the last set against
/// all previous sets successively remove duplicates, thus ensuring
/// the current set contains only the first instance of any filename
pub fn keep_unique_across_sets(all_dirs: &[HashSet<OsString>]) -> HashSet<OsString> {

    if all_dirs.is_empty() {
        return HashSet::new()
    }

    let last_index = all_dirs.len() - 1;

    let last_dir = all_dirs[last_index].clone();
    let previous_dirs = &all_dirs[0..last_index];

    // let (last_dir, previous_dirs) = &all_dirs.split_last().unwrap();

    previous_dirs.iter()
        .fold(last_dir, |accum: HashSet<OsString>, current_dir| {
            accum
                .difference(current_dir)
                .map(|d| d.clone())
                .collect::<HashSet<_>>()
        })
}

pub fn print_sets_with_index(msg: &str, set: &Vec<HashSet<OsString>>) {
    println!("{}:", msg);
    set.iter().enumerate()
        .for_each(|(ix, set)| println!("{:?} -> {:?}", ix, set));
}

pub fn print_progress(msg: String) {
    print!("{}", msg);
    let _ = std::io::stdout().flush();
}

pub fn get_string_char_count(s: String) -> usize {
    s.chars().count()
}

/// Convert bytes to an appropriate multiple (MB or GB) and append its unit
pub fn get_file_size_string(filesize: u64) -> String {
    match filesize {
        size if size <= 0 =>
            String::from("unknown"),
        size if size < 1024u64.pow(3) =>
            format!("{:.2} MB", (size as f64 / 1024u64.pow(2) as f64)),
        size =>
            format!("{:.2} GB", (size as f64/ 1024u64.pow(3) as f64))
    }
}