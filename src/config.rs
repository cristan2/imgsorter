use std::collections::{HashMap, HashSet};
use std::fs::DirEntry;
use std::path::PathBuf;
use std::time::Instant;
use std::{env, fs};

use crate::utils::*;

use toml::*;

// Config defaults
const DEFAULT_MIN_COUNT: i64 = 1;
const DEFAULT_ALWAYS_CREATE_DEVICE_DIR: bool = false;
const DEFAULT_COMPACTING_MIN_COUNT: usize = 0;
const DEFAULT_COPY: bool = true;
const DEFAULT_SILENT: bool = false;
const DEFAULT_DRY_RUN: bool = true;
const DEFAULT_VERBOSE: bool = false;
const DEFAULT_ALIGN_OUTPUT: bool = true;
const DEFAULT_SOURCE_RECURSIVE: bool = true;
const DEFAULT_INCLUDE_DEVICE_MAKE: bool = true;
static DEFAULT_ONEOFFS_DIR_NAME: &str = "Miscellaneous";

pub const IMAGE: &str = "image";
pub const VIDEO: &str = "video";
pub const AUDIO: &str = "audio";

// Unexposed defaults
const DBG_ON: bool = false;
const DEFAULT_TARGET_SUBDIR: &str = "imgsorted";
pub const DEFAULT_UNKNOWN_DEVICE_DIR_NAME: &str = "Unknown";
pub const DEFAULT_NO_DATE_STR: &str = "no date";
pub const DATE_DIR_FORMAT: &str = "%Y.%m.%d";

#[derive(Debug)]
pub struct Args {
    /// The directory or directories where the images to be sorted are located.
    /// If not provided, the current working dir will be used
    /// It's listed as a Vec<Vec<PathBuf>> to cover both cases when source_recursive is enabled
    /// or not - the outer Vec references the explicitly configured paths, while the inner Vec's
    /// will hold all subdirectories of those paths
    pub source_dir: Vec<Vec<PathBuf>>,

    /// Set to true only if we received a source_dir from the CLI
    /// Not exposed in config, only used during config parsing
    using_cli_source: bool,

    /// The recursive option might result in multiple sources (subdirs) being used even if
    ///   the configuration has a single source, so store this flag after checking sources
    /// Not exposed in config, internal only
    has_multiple_sources: bool,

    /// The directory where the images to be sorted will be moved.
    /// If not provided, the current working dir will be used.
    /// If the target does not exist, it will be created
    /// If the target *does* exist, a subdirectory called
    /// [DEFAULT_TARGET_SUBDIR] will be created and used,
    /// instead of directly placed in the target_dir
    pub target_dir: PathBuf,

    /// If this is enabled, descend into subdirectories recursively
    pub source_recursive: bool,

    /// The minimum number of files with the same date necessary
    /// for a dedicated subdir to be created
    pub min_files_per_dir: i64,

    /// Always create device subdirs, even if there's only a single one
    pub always_create_device_subdirs: bool,

    /// When doing a dry run, omit output for files with the same
    /// status if exceeding this threshold to save visual space
    pub compacting_threshold: usize,

    /// The name of the subdir which will hold files for any given date
    /// with less than or equal to the [min_files_per_dir] threshold
    pub oneoffs_dir_name: String,

    /// The current working directory
    pub cwd: PathBuf,

    /// Whether to ask for confirmation before processing files
    pub silent: bool,

    /// Whether files are copied instead of moved to the sorted subdirs
    pub copy_not_move: bool,

    /// Whether to do a simulation of the process, without writing any files
    /// This will display additional information, like the resulting dir structure
    /// including the total number of devices, files and file size
    pub dry_run: bool,

    /// Whether to print additional information during processing
    pub verbose: bool,

    /// Whether to align file lists for pretty outputs
    pub align_file_output: bool,

    /// Whether to print much more additional information during processing
    /// Not exposed in config, for dev-only
    pub debug: bool,

    /// Whether to also include device Make in addition to the device name
    pub include_device_make: bool,

    /// EXIF-retrieved names of device models can be replaced with custom names
    /// for improved clarity, e.g. "Samsung A41" instead of "SM-A415F"
    /// This is a simple mapping from device name to custom name.
    /// Keys should always be stored in lowercase for case-insensitive retrieval
    pub custom_device_names: HashMap<String, String>,

    /// This is not user-provided, it's used during parsing to build a set of
    /// "raw" device names, i.e. those that do not have a custom name defined
    pub non_custom_device_names: HashSet<String>,

    /// User-defined extensions for files to be processed which otherwise the program would skip
    pub custom_extensions: HashMap<String, Vec<String>>,
}

impl Args {
    /// Simple constructor using defaults: the CWD is the source
    /// directory and subdir will be created for the target paths
    pub fn new() -> Result<Args, std::io::Error> {
        let cwd = env::current_dir()?;

        let mut custom_extensions: HashMap<String, Vec<String>> = HashMap::new();
        custom_extensions.insert(IMAGE.to_lowercase(), Vec::new());
        custom_extensions.insert(VIDEO.to_lowercase(), Vec::new());
        custom_extensions.insert(AUDIO.to_lowercase(), Vec::new());

        Ok(Args {
            source_dir: vec![vec![cwd.clone()]],
            using_cli_source: false,
            has_multiple_sources: false,
            target_dir: cwd.clone().join(DEFAULT_TARGET_SUBDIR),
            source_recursive: DEFAULT_SOURCE_RECURSIVE,
            min_files_per_dir: DEFAULT_MIN_COUNT,
            always_create_device_subdirs: DEFAULT_ALWAYS_CREATE_DEVICE_DIR,
            compacting_threshold: DEFAULT_COMPACTING_MIN_COUNT,
            oneoffs_dir_name: String::from(DEFAULT_ONEOFFS_DIR_NAME),
            cwd,
            silent: DEFAULT_SILENT,
            copy_not_move: DEFAULT_COPY,
            dry_run: DEFAULT_DRY_RUN,
            verbose: DEFAULT_VERBOSE,
            align_file_output: DEFAULT_ALIGN_OUTPUT,
            debug: DBG_ON,
            include_device_make: DEFAULT_INCLUDE_DEVICE_MAKE,
            custom_device_names: HashMap::new(),
            non_custom_device_names: HashSet::new(),
            custom_extensions,
        })
    }

    /// Several ways to launch the program
    /// - edit registry - use context menu in dir - imgsort current dir - sends current dir as path override
    /// - add program dir to path - use terminal - navigate to any dir
    ///   - launch using program name only - uses source in config file
    ///   - launch using program name and "." - uses current dir as path override
    ///   - launch using program name and any path - uses that path as path override
    /// In all cases, config file should be read from the executable location, if present,
    /// otherwise fallback to relative path, which likely will fall as well (should only work for debug builds in IDE)\
    /// and will end up not using the config file and just use the preset defaults
    pub fn new_from_toml(config_file: &str) -> Result<Args, std::io::Error> {
        let mut args = Args::new()?;

        // Temporarily store missing keys and other errors so we can print them
        // once we've checked all config values and determined verbosity option
        let mut verbose_messages: Vec<String> = Vec::new();
        let mut missing_vals: Vec<String> = Vec::new();
        let mut invalid_vals: Vec<(String, String)> = Vec::new();

        let (config_file_path, message) = get_config_file_path(config_file);
        verbose_messages.push(message);

        // The program can receive a source path from the CLI, either a path directly provided by user
        // or the current working directory from the system when launched from the Windows explorer context menu
        // If we receive this, use it as both the source and target dirs and toggle the [using_cli_source] flag to skip
        // reading the source and target values from config. Otherwise, do nothing and fallback to config.
        if let Some(cli_source) = get_cli_source_path() {
            let cli_src_path = vec![PathBuf::from(cli_source.clone())];
            match validate_source_paths(cli_src_path) {
                Ok((valid_paths, _)) => {
                    println!("Using source path at: {}", &cli_source);
                    args.set_source_paths(vec![valid_paths]);
                    args.set_target_dir(cli_source);
                    args.using_cli_source = true;
                }
                _ => {
                    let message = ColoredString::orange(format!(
                        "User provided path is not valid: {}", &cli_source).as_str());
                    verbose_messages.push(message);
                }
            }
        }

        type TomlMap = toml::map::Map<String, toml::Value>;

        fn get_boolean_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<bool> {
            let bool_opt = toml_table
                .get(key)
                .map(|toml_value| toml_value.as_bool())
                .flatten();

            if bool_opt.is_none() { missing_vals.push(String::from(key))  };
            bool_opt
        }

        // Same as [get_boolean_value], but don't print if missing.
        // Used for unexposed config values
        fn get_boolean_value_silent(toml_table: &TomlMap, key: &str) -> Option<bool> {
            toml_table
                .get(key)
                .map(|toml_value| toml_value.as_bool())
                .flatten()
        }

        // Will always return a positive integer. If the number is negative, will return None
        fn get_positive_integer_value(
            toml_table: &TomlMap,
            key: &str,
            missing_vals: &mut Vec<String>,
            invalid_vals: &mut Vec<(String, String)>,
        ) -> Option<i64> {
            let value = toml_table
                .get(key)
                .map(|toml_value| toml_value.as_integer())
                .flatten();

            match value {
                None => {
                    missing_vals.push(String::from(key));
                    None
                }
                Some(x) if x < 0 => {
                    invalid_vals.push((
                        String::from(key),
                        String::from("Number must be greater than 0"),
                    ));
                    None
                }
                Some(x) => Some(x),
            }
        }

        fn get_string_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<String> {
            let string_opt = toml_table
                .get(key)
                .map(|toml_value| toml_value.as_str())
                .flatten()
                .map(String::from);

            if string_opt.is_none() { missing_vals.push(String::from(key)) };
            string_opt
        }

        fn get_array_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<Vec<String>> {
            let vec_opt = toml_table
                .get(key)
                .map(|toml_value| toml_value.as_array())
                .flatten()
                .map(|strings_vec| {
                    strings_vec
                        .iter()
                        .flat_map(|value| value.as_str())
                        .map(String::from)
                        .collect::<Vec<_>>()
                });

            if vec_opt.is_none() { missing_vals.push(String::from(key)) };
            vec_opt
        }

        fn get_strings_dict_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<HashMap<String, String>> {
            let dict_opt = toml_table
                .get(key)
                .map(|toml_dict|{ toml_dict.as_table()})
                .flatten()
                .map(|key_values| {
                    key_values
                        .into_iter()
                        .map(|(dict_key, dict_value)| (dict_key, dict_value.as_str()))
                        .filter(|(_, dict_value)| dict_value.is_some())
                        .map(|(dict_key, dict_value)|
                            // When the dict_key is used as key in the resulting hashmap,
                            // transform it to lowercase to allow case-insensitive retrievals
                            (dict_key.to_lowercase(), String::from(dict_value.unwrap())))
                        .collect::<HashMap<String, String>>()
                });

            if dict_opt.is_none() { missing_vals.push(String::from(key)) };
            dict_opt
        }

        fn get_paths(path_strs: Vec<String>) -> Vec<PathBuf> {
            path_strs.iter().map(PathBuf::from).collect::<Vec<_>>()
        }

        fn vec_to_lowercase(vec_strings: Vec<String>) -> Vec<String> {
            vec_strings.into_iter().map(|s| s.to_lowercase()).collect()
        }

        match fs::read_to_string(&config_file_path) {
            Ok(file_contents) => {
                println!("Using config file at: {}", &config_file_path.display().to_string());
                match file_contents.parse::<Value>() {
                    Ok(raw_toml) => {
                        match raw_toml.as_table() {
                            Some(toml_content) => {

                                /* --- Parse source/target folders --- */

                                match toml_content.get("folders") {
                                    Some(folders_opt) => {
                                        if let Some(folders) = folders_opt.as_table() {

                                            fn print_source_folders_help() {
                                                println!("{}",
                                                    format!(
                                                        // TODO 5f: use OS-specific path separators
                                                        "{}\n-----{}\n{}\n{}\n{}\n{}\n-----",
                                                        "Edit imgsorter.toml and add valid source folders like this:",
                                                        "[folders]",
                                                        "source_dirs = [",
                                                        "  'D:\\Example dir\\Pictures',",
                                                        "  'E:\\My dir\\Pictures',",
                                                        "]"));
                                            }

                                            // Use config source and target paths only if we didn't receive a CLI path override
                                            if !args.using_cli_source {
                                                // If no valid source paths are found, use current working directory and print a red warning
                                                // otherwise, use whatever sources are valid and print a yellow warning for the rest
                                                if let Some(source_paths) = get_array_value(folders, "source_dirs", &mut missing_vals) {

                                                    println!("Using source paths from configuration file.");

                                                    match validate_source_paths(get_paths(source_paths)) {
                                                        Err(all_invalid_sources) => {
                                                            println!("{}", ColoredString::red(
                                                                format!(
                                                                    "All source folders are invalid!\n {}",
                                                                    paths_to_str(all_invalid_sources)).as_str()));
                                                            print_source_folders_help();
                                                            // The cwd previously set as default value will remain used
                                                            println!("Using current working directory for now: {}", args.source_dir[0][0].display());
                                                        }

                                                        Ok((valid_paths, invalid_paths)) => {
                                                            if !invalid_paths.is_empty() {
                                                                println!("{}", ColoredString::orange(
                                                                    format!(
                                                                        "Some source folders were invalid and were ignored:\n {}",
                                                                        paths_to_str(invalid_paths)).as_str()));
                                                                print_source_folders_help()
                                                            }

                                                            let path_vecs: Vec<Vec<PathBuf>> =
                                                                valid_paths
                                                                    .into_iter()
                                                                    .map(|path|vec![path])
                                                                    .collect();

                                                            args.set_source_paths(path_vecs);
                                                        }
                                                    }
                                                } else {
                                                    println!("{}", ColoredString::red("No source folders found!"));
                                                    print_source_folders_help();
                                                    println!("Using current working directory for now: {}", args.source_dir[0][0].display());
                                                }

                                                // Not exposed in config; use for dev only
                                                // source_subdir = 'test_pics'
                                                if let Some(source_subdir) = get_string_value(folders, "source_subdir", &mut missing_vals) {
                                                    // get_string_value already filters out empty strings, but just to be safe
                                                    if !source_subdir.is_empty() {
                                                        args.append_source_subdir(source_subdir.as_str());
                                                    }
                                                }

                                                if let Some(target_dir) = get_string_value(folders, "target_dir", &mut missing_vals) {
                                                    // get_string_value already filters out empty strings, but just to be safe
                                                    if !target_dir.is_empty() {
                                                        args.set_target_dir(target_dir);
                                                    }
                                                }
                                            } // end if !args.using_cli_source

                                            if let Some(min_files_per_dir) = get_positive_integer_value(folders, "min_files_per_dir", &mut missing_vals, &mut invalid_vals) {
                                                args.min_files_per_dir = min_files_per_dir;
                                            }

                                            if let Some(compacting_threshold) = get_positive_integer_value(folders, "min_files_before_compacting_output", &mut missing_vals, &mut invalid_vals) {
                                                args.compacting_threshold = compacting_threshold as usize;
                                            }

                                            if let Some(oneoffs_dir_name) = get_string_value(folders, "target_oneoffs_subdir_name", &mut missing_vals) {
                                                // get_string_value already filters out empty strings, but just to be safe
                                                if !oneoffs_dir_name.is_empty() {
                                                    args.oneoffs_dir_name = oneoffs_dir_name;
                                                }
                                            }
                                        } // end if let Some(folders)
                                    } // end Some(folders_opt)
                                    None =>
                                      missing_vals.push(String::from("folders"))
                                } // end config folders

                                /* --- Parse options --- */

                                match toml_content.get("options") {
                                    Some(options_opt) => {
                                        if let Some(options) = options_opt.as_table() {

                                            // Not exposed in config; use for dev only
                                            // debug_on = true
                                            if let Some(debug_on) = get_boolean_value_silent(options, "debug") {
                                                args.debug = debug_on;
                                                args.verbose = debug_on;
                                            } else if let Some(verbose) = get_boolean_value(options, "verbose", &mut missing_vals) {
                                                args.verbose = verbose;
                                            }

                                            if let Some(source_recursive) = get_boolean_value(options, "source_recursive", &mut missing_vals) {
                                                args.source_recursive = source_recursive;
                                            }

                                            if let Some(dry_run) = get_boolean_value(options, "dry_run", &mut missing_vals) {
                                                args.dry_run = dry_run;
                                            }

                                            if let Some(align_file_output) = get_boolean_value(options, "align_file_output", &mut missing_vals) {
                                                args.align_file_output = align_file_output;
                                            }

                                            if let Some(include_device_make) = get_boolean_value(options, "include_device_make", &mut missing_vals) {
                                                args.include_device_make = include_device_make;
                                            }

                                            if let Some(always_create_device_subdirs) = get_boolean_value(options, "always_create_device_subdirs", &mut missing_vals) {
                                                args.always_create_device_subdirs = always_create_device_subdirs;
                                            }

                                            if let Some(copy_not_move) = get_boolean_value(options, "copy_not_move", &mut missing_vals) {
                                                args.copy_not_move = copy_not_move;
                                            }

                                            if let Some(silent) = get_boolean_value(options, "silent", &mut missing_vals) {
                                                args.silent = silent;
                                            }
                                        }
                                    }
                                    None =>
                                        missing_vals.push(String::from("options")),
                                } // end config options

                                /* --- Parse custom data --- */

                                match toml_content.get("custom") {
                                    Some(custom_data_opt) => {
                                        if let Some(custom_data) = custom_data_opt.as_table() {
                                            if let Some(devices_dict) = get_strings_dict_value(custom_data, "devices", &mut missing_vals) {
                                                args.custom_device_names = devices_dict;
                                            }

                                            match custom_data.get("extensions") {
                                                Some(custom_extensions_opt) => {
                                                    if let Some(custom_extensions) = custom_extensions_opt.as_table() {

                                                        if let Some(custom_image_ext) = get_array_value(custom_extensions, "image", &mut missing_vals) {
                                                            args.custom_extensions.insert(IMAGE.to_lowercase(), vec_to_lowercase(custom_image_ext));
                                                        }

                                                        if let Some(custom_video_ext) = get_array_value(custom_extensions, "video", &mut missing_vals) {
                                                            args.custom_extensions.insert(VIDEO.to_lowercase(), vec_to_lowercase(custom_video_ext));
                                                        }

                                                        if let Some(custom_audio_ext) = get_array_value(custom_extensions, "audio", &mut missing_vals) {
                                                            args.custom_extensions.insert(AUDIO.to_lowercase(), vec_to_lowercase(custom_audio_ext));
                                                        }
                                                    } // end if let Some(custom_extensions)
                                                }
                                                None =>
                                                    missing_vals.push(String::from("extensions"))
                                            } // end match extensions
                                        } // end if let Some(custom_data)
                                    } // if let Some(custom_data_opt)
                                    None =>
                                        missing_vals.push(String::from("custom")),
                                } // end config custom data
                            }
                            None => {
                                println!("Could not parse TOML into a key-value object");
                            }
                        } // end reading raw toml data
                    }
                    Err(err) => {
                        println!("{}", ColoredString::red(format!("Error: {}", err).as_str()));
                        println!("{}", ColoredString::red(
                            "Could not parse config file, continuing with defaults."));
                    }
                } // end reading config file contents
            }
            Err(e) => {
                println!("{}", ColoredString::red(format!(
                        "Could not read config file at {}. Continuing with defaults.",
                        &config_file_path.display().to_string())
                    .as_str()));
                eprintln!("{}", e);
            }
        };

        // Print missing and invalid values
        if args.verbose {
            missing_vals.iter().for_each(|key|
                println!("> Config key '{}' is empty, invalid or missing. Using preset default.", key)
            );

            invalid_vals.iter().for_each(|(key, message)| {
                println!("> Config key '{}' is invalid: {}", key, message)
            });
        }

        // Once all source folders and options are read, check if we need to
        // recursively read subdirectories and set all sources
        if args.source_recursive {

            if args.verbose { println!("> Fetching source directories list recursively..."); }
            let _time_fetching_dirs = Instant::now();

            let new_source_dirs = walk_source_dirs_recursively(&args);
            if new_source_dirs.is_empty() {
                // This shouldn't happen, but let's be sure
                panic!("Source folders are empty or don't exist");
            } else {
                if args.verbose { println!("> Setting {} source folder(s)", new_source_dirs.len()); }
                args.set_source_paths(new_source_dirs);
            }

            // TODO 3d: import FileStats and reenable this
            // stats.set_time_fetch_dirs(_time_fetching_dirs.elapsed());
        }

        // The recursive option above might result in multiple sources being defined,
        // even if the configuration has a single source, so check this now and store the result
        let src_len: usize = args.source_dir.iter().map(|v|v.len()).sum();
        args.has_multiple_sources = src_len > 1;

        Ok(args)
    }

    fn set_source_paths(&mut self, sources: Vec<Vec<PathBuf>>) {
        if !(sources.is_empty() || sources.iter().all(|v|v.is_empty())) {
            self.source_dir = sources;
        }
    }

    // Create the target path from the provided target_path_str
    // If the path already exists, create subdirectory DEFAULT_TARGET_SUBDIR inside it
    fn set_target_dir(&mut self, target_path_str: String) {
        let target_path = PathBuf::from(target_path_str);
        self.target_dir = if target_path.exists() {
            target_path.join(DEFAULT_TARGET_SUBDIR)
        } else {
            target_path
        }
    }

    fn append_source_subdir(&mut self, subdir: &str) {
        if self.source_dir.len() == 1 && self.source_dir[0].len() == 1{
            self.source_dir[0][0].push(subdir);
        }
    }

    pub fn has_multiple_sources(&self) -> bool {
        self.has_multiple_sources
    }

    pub fn is_compacting_enabled(&self) -> bool {
        self.compacting_threshold > 0
    }
}

fn get_config_file_path(config_file_name: &str) -> (PathBuf, String) {
    let cfg_relative_path = PathBuf::from(config_file_name);

    match get_program_executable_path() {
        Ok(path) => {
            let config_path = path.join(config_file_name);
            if config_path.exists() {
                let message = format!("Found config file at: {}", &path.display().to_string());
                (config_path, message)
            } else {
                let message = ColoredString::orange(format!(
                    "Trying relative path. Config file not found at: {}.", &path.display().to_string()).as_str());
                (cfg_relative_path, message)
            }
        }
        Err(path_reading_err) => {
            (cfg_relative_path, path_reading_err)
        }
    }
}

fn get_cli_source_path() -> Option<String> {
    let cli_args: Vec<String> = env::args().collect();

    cli_args
        .get(1)
        .cloned()
}

fn get_program_executable_path() -> Result<PathBuf, String> {
    match std::env::current_exe() {
        Ok(executable_path) => {
            match executable_path.parent() {
                Some(path) =>
                    Ok(path.to_path_buf()),
                None => {
                    Err(ColoredString::red("Could not extract program."))
                }
            }
        },
        Err(e) => {
            eprintln!("{}", e);
            Err(ColoredString::red("Could not read path for program executable."))
        },
    }
}

/// Check if the provided sources exist and return a `valid_path`
/// Vec only if there's at least one valid source path
fn validate_source_paths(sources: Vec<PathBuf>) -> Result<(Vec<PathBuf>, Vec<PathBuf>), Vec<PathBuf>> {
    let (valid_paths, invalid_paths): (Vec<PathBuf>, Vec<PathBuf>) =
        sources.into_iter().partition(|path| path.exists());

    if valid_paths.is_empty() {
        Err(invalid_paths)
    } else {
        Ok((valid_paths, invalid_paths))
    }
}

fn paths_to_str(paths: Vec<PathBuf>) -> String {
    paths
        .iter()
        .flat_map(|s| s.to_str())
        .collect::<Vec<_>>()
        .join("\n ")
}

/// For each configured source directory, read all its inner subdirectories
/// recursively into a separate Vec, so the end result will be a 2D Vec where
/// the outer elements hold all subdirs of each of the configured source dirs,
/// while the inner elements represent the actual subdir paths, e.g.:
/// ```
/// [
///   [src_dir_1, src_dir_1/subdir1, src_dir_1/subdir2],
///   [src_dir_2, src_dir_2/subdir1, src_dir_2/subdir2/another_subdir_level],
/// ]
/// ```
fn walk_source_dirs_recursively(args: &Args) -> Vec<Vec<PathBuf>> {
    fn walk_dir(
        source_dir: PathBuf,
        vec_accum: &mut Vec<PathBuf>,
        args: &Args,
    ) -> Result<(), std::io::Error> {
        if args.verbose {
            println!("> Reading '{}'", &source_dir.display().to_string());
        }

        let subdirs: Vec<DirEntry> = fs::read_dir(&source_dir)?
            .into_iter()
            .filter_map(|s| s.ok())
            .filter(|entry| entry.path().is_dir())
            .collect::<Vec<_>>();

        vec_accum.push(source_dir);

        if !subdirs.is_empty() {
            subdirs.iter().for_each(|dir_entry| {
                let _ = walk_dir(dir_entry.path(), vec_accum, args);
            });
        };

        Ok(())
    }

    args.source_dir.clone()
        .into_iter()
        .flat_map(|source_dir|
            source_dir
                .into_iter()
                .map(|d| {
                    let mut start_vec: Vec<PathBuf> = Vec::new();
                    walk_dir(d, &mut start_vec, args).ok();
                    start_vec
                })
        )
        .collect()
}
