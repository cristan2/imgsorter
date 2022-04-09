use std::path::PathBuf;
use std::{fs, env};
use std::collections::HashMap;
use std::fs::DirEntry;
use std::time::Instant;

use crate::utils::*;

use toml::*;

// Config defaults
const DEFAULT_MIN_COUNT: i64 = 1;
const DEFAULT_COPY: bool = true;
const DEFAULT_SILENT: bool = false;
const DEFAULT_DRY_RUN: bool = false;
const DEFAULT_VERBOSE: bool = false;
const DEFAULT_SOURCE_RECURSIVE: bool = false;
static DEFAULT_ONEOFFS_DIR_NAME: &str = "Miscellaneous";

pub const IMAGE: &str = "image";
pub const VIDEO: &str = "video";
pub const AUDIO: &str = "audio";

// Unexposed defaults
const DBG_ON: bool = false;
const DEFAULT_TARGET_SUBDIR: &str = "imgsorted";

#[derive(Debug)]
pub struct Args {
    /// The directory or directories where the images to be sorted are located.
    /// If not provided, the current working dir will be used
    pub source_dir: Vec<PathBuf>,

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

    /// Whether to print much more additional information during processing
    /// Not exposed in config, for dev-only
    pub debug: bool,

    /// EXIF-retrieved names of device models can be replaced with custom names
    /// for improved clarity, e.g. "Samsung A41" instead of "SM-A415F"
    /// This is a simple mapping from device name to custom name.
    /// Keys should always be stored in lowercase for case-insensitive retrieval
    pub custom_device_names: HashMap<String, String>,

    /// User-defined extensions for files to be processed which otherwise the program would skip
    pub custom_extensions: HashMap<String, Vec<String>>
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

        Ok(
            Args {
                source_dir: vec![cwd.clone()],
                target_dir: cwd.clone().join(DEFAULT_TARGET_SUBDIR),
                source_recursive: DEFAULT_SOURCE_RECURSIVE,
                min_files_per_dir: DEFAULT_MIN_COUNT,
                oneoffs_dir_name: String::from(DEFAULT_ONEOFFS_DIR_NAME),
                cwd,
                silent: DEFAULT_SILENT,
                copy_not_move: DEFAULT_COPY,
                dry_run: DEFAULT_DRY_RUN,
                verbose: DEFAULT_VERBOSE,
                debug: DBG_ON,
                custom_device_names: HashMap::new(),
                custom_extensions
            })
    }

    // fn new_with_options(
    //     // Full path from where to read images to be sorted
    //     source: Option<String>,
    //     // Subdir inside the CWD from where to read images to be sorted
    //     // Note: if `source` is provided, this is ignored
    //     cwd_source_subdir: Option<String>,
    //     // Full path where the sorted images will be moved
    //     target: Option<String>,
    //     source_recursive: Option<bool>,
    //     // Subdir inside the CWD where the sorted images will be moved
    //     // Note: if `target` is provided, this is ignored
    //     cwd_target_subdir: Option<String>,
    //     min_files: Option<i64>,
    //     oneoffs_dir_name: Option<String>,
    //     silent: Option<bool>,
    //     copy_not_move: Option<bool>,
    //     dry_run: Option<bool>,
    //     debug_on: Option<bool>
    // ) -> Result<Args, std::io::Error> {
    //
    //     fn create_path(provided_path: Option<String>, path_subdir: Option<String>, cwd: &PathBuf) -> PathBuf {
    //         match provided_path {
    //             // if a full path has been provided, use that
    //             Some(path) =>
    //                 PathBuf::from(path),
    //             // otherwise, use the cwd...
    //             None => {
    //                 // but create a subdir if one was provided
    //                 match path_subdir {
    //                     Some(subdir) =>
    //                         cwd.join(subdir),
    //                     None =>
    //                         cwd.clone()
    //                 }
    //             }
    //         }
    //     }
    //
    //     let cwd = env::current_dir()?;
    //
    //     Ok(
    //         Args {
    //             source_dir: vec![create_path(source, cwd_source_subdir, &cwd)],
    //             target_dir: create_path(
    //                 target,
    //                 cwd_target_subdir.or(Some(String::from(DEFAULT_TARGET_SUBDIR))),
    //                 &cwd),
    //             source_recursive: source_recursive.unwrap_or(DEFAULT_SOURCE_RECURSIVE),
    //             min_files_per_dir: min_files.unwrap_or(DEFAULT_MIN_COUNT),
    //             oneoffs_dir_name: oneoffs_dir_name.unwrap_or(String::from(DEFAULT_ONEOFFS_DIR_NAME)),
    //             cwd,
    //             silent: silent.unwrap_or(DEFAULT_SILENT),
    //             copy_not_move: copy_not_move.unwrap_or(DEFAULT_COPY),
    //             dry_run: dry_run.unwrap_or(DEFAULT_DRY_RUN),
    //             debug: debug_on.unwrap_or(DBG_ON),
    //         }
    //     )
    // }

    pub fn new_from_toml(config_file: &str) -> Result<Args, std::io::Error> {
        let mut args = Args::new()?;

        type TomlMap = toml::map::Map<String, toml::Value>;

        // temporarily store missing keys so we can print them once we've checked all config values
        let mut missing_vals: Vec<String> = Vec::new();
        let mut invalid_vals: Vec<(String, String)> = Vec::new();

        fn get_boolean_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<bool> {
            let bool_opt = toml_table.get(key)
                .map(|toml_value| toml_value.as_bool())
                .flatten();
            
            if bool_opt.is_none() { missing_vals.push(String::from(key))  };
            bool_opt
        }

        // Same as [get_boolean_value], but don't print if missing.
        // Used for unexposed config values
        fn get_boolean_value_silent(toml_table: &TomlMap, key: &str) -> Option<bool> {
            toml_table.get(key)
                .map(|toml_value| toml_value.as_bool())
                .flatten()
        }

        // Will always return a positive integer. If the number is negative, will return None
        fn get_positive_integer_value(
            toml_table: &TomlMap,
            key: &str,
            missing_vals: &mut Vec<String>,
            invalid_vals: &mut Vec<(String, String)>
        ) -> Option<i64> {
            let value = toml_table.get(key)
                .map(|toml_value| toml_value.as_integer())
                .flatten();

            match value {
                None => {
                    missing_vals.push(String::from(key));
                    None
                },
                Some(x) if x < 0 => {
                    invalid_vals.push(
                        (String::from(key), String::from("Number must be greater than 0"))
                    );
                    None
                },
                Some(x) => Some(x)
            }
        }

        fn get_string_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<String> {
            let string_opt = toml_table.get(key)
                .map(|toml_value| toml_value.as_str())
                .flatten()
                .map(|str_val| String::from(str_val));

            if string_opt.is_none() { missing_vals.push(String::from(key)) };
            string_opt
        }

        fn get_array_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<Vec<String>> {
            let vec_opt = toml_table.get(key)
                .map(|toml_value| toml_value.as_array())
                .flatten()
                .map(|strings_vec|{
                    strings_vec.iter()
                        .flat_map(|value| value.as_str())
                        .map(|s|String::from(s))
                        .collect::<Vec<_>>()
                });

            if vec_opt.is_none() { missing_vals.push(String::from(key)) };
            vec_opt
        }

        fn get_strings_dict_value(toml_table: &TomlMap, key: &str, missing_vals: &mut Vec<String>) -> Option<HashMap<String, String>> {
            let dict_opt = toml_table.get(key)
                .map(|toml_dict|{ toml_dict.as_table()})
                .flatten()
                .map(|key_values| {
                    key_values
                        .into_iter()
                        .map(|(dict_key, dict_value)|
                            (dict_key, dict_value.as_str()))
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
            path_strs
                .iter()
                .map(|s|PathBuf::from(s))
                .collect::<Vec<_>>()
        }

        fn vec_to_lowercase(vec_strings: Vec<String>) -> Vec<String> {
            vec_strings.into_iter().map(|s|s.to_lowercase()).collect()
        }

        match fs::read_to_string(config_file) {
            Ok(file_contents) => {
                match file_contents.parse::<Value>() {
                    Ok(raw_toml) => {

                        match raw_toml.as_table() {

                            Some(toml_content) => {

                                /* --- Parse source/target folders --- */

                                match toml_content.get("folders") {
                                    Some(folders_opt) => {
                                        if let Some(folders) = folders_opt.as_table() {

                                            // args.set_source_paths will return an error and we exit if no valid
                                            // source directories are found - there's nothing to do without a source
                                            if let Some(source_paths) = get_array_value(&folders, "source_dirs", &mut missing_vals) {
                                                args.set_source_paths(get_paths(source_paths))?;
                                            // TODO run in CWD??
                                            } else {
                                                println!("{}", ColoredString::red(
                                                    format!(
                                                       "Config file has no valid source folders!\n{}\n{}\n{}\n{}\n{}",
                                                       "Edit imgsorter.toml and add the following lines, filling in valid paths:",
                                                       "source_dirs = [",
                                                       "  'D:\\Example dir\\Pictures',",
                                                       "  'E:\\My dir\\Pictures',",
                                                       "]").as_str()));
                                                return Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                                            }

                                            // Not exposed in config; use for dev only
                                            // source_subdir = 'test_pics'
                                            if let Some(source_subdir) = get_string_value(&folders, "source_subdir", &mut missing_vals) {
                                                // get_string_value already filters out empty strings, but just to be safe
                                                if !source_subdir.is_empty() {
                                                    args.append_source_subdir(source_subdir.as_str());
                                                }
                                            }

                                            if let Some(target_dir) = get_string_value(&folders, "target_dir", &mut missing_vals) {
                                                // get_string_value already filters out empty strings, but just to be safe
                                                if !target_dir.is_empty() {
                                                    args.set_target_dir(target_dir);
                                                }
                                            }

                                            if let Some(min_files_per_dir) = get_positive_integer_value(&folders, "min_files_per_dir", &mut missing_vals, &mut invalid_vals) {
                                                args.min_files_per_dir = min_files_per_dir;
                                            }

                                            if let Some(oneoffs_dir_name) = get_string_value(&folders, "target_oneoffs_subdir_name", &mut missing_vals) {
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

                                            if let Some(source_recursive) = get_boolean_value(&options, "source_recursive", &mut missing_vals) {
                                                args.source_recursive = source_recursive;
                                            }

                                            if let Some(dry_run) = get_boolean_value(&options, "dry_run", &mut missing_vals) {
                                                args.dry_run = dry_run;
                                            }

                                            if let Some(verbose) = get_boolean_value(&options, "verbose", &mut missing_vals) {
                                                args.verbose = verbose;
                                            }

                                            if let Some(copy_not_move) = get_boolean_value(&options, "copy_not_move", &mut missing_vals) {
                                                args.copy_not_move = copy_not_move;
                                            }

                                            if let Some(silent) = get_boolean_value(&options, "silent", &mut missing_vals) {
                                                args.silent = silent;
                                            }

                                            // Not exposed in config; use for dev only
                                            // debug_on = true
                                            if let Some(debug_on) = get_boolean_value_silent(&options, "debug_on") {
                                                args.debug = debug_on;
                                            }
                                        }
                                    },
                                    None =>
                                        missing_vals.push(String::from("options"))

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

                                                        if let Some(custom_image_ext) = get_array_value(&custom_extensions, "image", &mut missing_vals) {
                                                            args.custom_extensions.insert(IMAGE.to_lowercase(), vec_to_lowercase(custom_image_ext));
                                                        }

                                                        if let Some(custom_video_ext) = get_array_value(&custom_extensions, "video", &mut missing_vals) {
                                                            args.custom_extensions.insert(VIDEO.to_lowercase(), vec_to_lowercase(custom_video_ext));
                                                        }

                                                        if let Some(custom_audio_ext) = get_array_value(&custom_extensions, "audio", &mut missing_vals) {
                                                            args.custom_extensions.insert(AUDIO.to_lowercase(), vec_to_lowercase(custom_audio_ext));
                                                        }
                                                    } // end if let Some(custom_extensions)
                                                },
                                                None =>
                                                    missing_vals.push(String::from("extensions"))
                                            } // end match extensions

                                        } // end if let Some(custom_data)
                                    } // if let Some(custom_data_opt)
                                    None =>
                                        missing_vals.push(String::from("custom"))
                                } // end config custom data
                            },
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
                    "Could not read config file {}. Continuing with defaults.", config_file).as_str()));
                eprintln!("{}", e);
            }
        };

        // Print missing and invalid values
        if args.verbose {
            missing_vals.iter().for_each(|key|
                println!("> Config key '{}' is empty, invalid or missing. Using preset default.", key)
            );

            invalid_vals.iter().for_each(|(key, message)|
                println!("> Config key '{}' is invalid: {}", key, message)
            );
        }


        // Once all source folders and options are read, check if we need to
        // recursively read subdirectories and set all sources

        if args.source_recursive {

            if args.verbose { println!("> Fetching source directories list recursively..."); }
            let time_fetching_dirs = Instant::now();

            let new_source_dirs = walk_source_dirs_recursively(&args);
            if new_source_dirs.is_empty() {
                // TODO replace with Err
                panic!("Source folders are empty or don't exist");
            } else {
                if args.verbose { println!("> Setting {} source folder(s)", new_source_dirs.len()); }
                args.source_dir = new_source_dirs;
            }

            // TODO 3d: import FileStats and reenable this
            // stats.set_time_fetch_dirs(time_fetching_dirs.elapsed());
        }

        Ok(args)
    }

    /// Change the source directory. This will also change the target
    /// directory to a subdir in the same directory. To set a different
    /// target directory, use [set_target_dir]
    // fn set_source_dir(mut self, source: &str) -> Args {
    //     let new_path = PathBuf::from(source);
    //     self.target_dir = new_path.clone().join(DEFAULT_TARGET_SUBDIR);
    //     self.source_dir = vec![new_path];
    //     self
    // }

    /// Change the source directory. This will also change the target
    /// directory to a subdir in the same directory. To set a different
    /// target directory, use [set_target_dir]
    // fn set_source_dirs(mut self, sources: Vec<&str>) -> Args {
    //     let source_paths = sources.iter()
    //         .map(|src_dir| PathBuf::from(src_dir))
    //         .collect::<Vec<PathBuf>>();
    // 
    //     // self.target_dir = new_path.clone().join(DEFAULT_TARGET_SUBDIR);
    //     self.source_dir = source_paths;
    //     self
    // }

    // fn add_source_dir(mut self, src_dir: &str) -> Args {
    //     self.source_dir.push(PathBuf::from(src_dir));
    //     self
    // }

    fn set_source_paths(&mut self, sources: Vec<PathBuf>) -> Result<(), std::io::Error> {
        let (valid_paths, invalid_paths): (Vec<PathBuf>, Vec<PathBuf>) = sources
            .into_iter()
            .partition(|path| path.exists());

        let list_of_invalid = invalid_paths
            .iter()
            .flat_map(|s|s.to_str())
            .collect::<Vec<_>>()
            .join("\n  ");

        if valid_paths.is_empty() {
            println!("{}", ColoredString::red(
                format!(
                    "Invalid source folders!\n> {}",
                    list_of_invalid).as_str()));
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        } else {
            if !list_of_invalid.is_empty() {
                println!("{}", ColoredString::orange(
                    format!(
                        "> Some source folders were invalid and were ignored:\n  {}",
                        list_of_invalid).as_str()));
            }
            self.source_dir = valid_paths;
            Ok(())
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
        if self.source_dir.len() == 1 {
            self.source_dir[0].push(subdir);
        }
    }

    // fn append_target_subdir(mut self, subdir: &str) -> Args {
    //     self.target_dir.push(subdir);
    //     self
    // }

    pub fn has_multiple_sources(&self) -> bool {
        self.source_dir.len() > 1
    }
}

fn walk_source_dirs_recursively(args: &Args) -> Vec<PathBuf> {

    fn walk_dir(
        source_dir: PathBuf,
        vec_accum: &mut Vec<PathBuf>,
        args: &Args
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
            subdirs
                .iter()
                .for_each(|dir_entry| {
                    let _ = walk_dir(dir_entry.path(), vec_accum, args);
                });
        };

        Ok(())
    }

    let mut new_source_dirs = Vec::new();

    args.source_dir.clone()
        .into_iter()
        .for_each(|d| {
            walk_dir(d, &mut new_source_dirs, args).ok();
        });

    new_source_dirs
}