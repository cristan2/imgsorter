use std::path::PathBuf;
use std::{fs, env};
use std::collections::HashMap;

use crate::utils::*;

use toml::*;

// Config defaults
const DEFAULT_MIN_COUNT: i64 = 1;
const DEFAULT_COPY: bool = true;
const DEFAULT_SILENT: bool = false;
const DEFAULT_DRY_RUN: bool = false;
const DEFAULT_SOURCE_RECURSIVE: bool = false;
static DEFAULT_ONEOFFS_DIR_NAME: &str = "Miscellaneous";

// Unexposed defaults
pub const DBG_ON: bool = false;
const DEFAULT_TARGET_SUBDIR: &'static str = "imgsorted";

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
    pub debug: bool,

    /// EXIF-retrieved names of device models can be replaced with custom names
    /// for improved clarity, e.g. "Samsung A41" instead of "SM-A415F"
    /// This is a simple mapping from device name to custom name.
    /// Keys should always be stored in lowercase for case-insensitive retrieval
    pub custom_device_names: HashMap<String, String>
}

impl Args {

    /// Simple constructor using defaults: the CWD is the source
    /// directory and subdir will be created for the target paths
    pub fn new() -> Result<Args, std::io::Error> {

        let cwd = env::current_dir()?;

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
                debug: DBG_ON,
                custom_device_names: HashMap::new()
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

        fn print_missing_value(value: &str) {
            // TODO if debug_on is read from args, this should be set first
            if DBG_ON {
                println!("> Config key '{}' is empty, invalid or missing. Using preset default.", value);
            }
        }

        fn print_invalid_value(key: &str, message: &str) {
            println!("> Config key '{}' is invalid: {}", key, message);
        }

        fn get_boolean_value(toml_table: &TomlMap, key: &str) -> Option<bool> {
            let bool_opt = toml_table.get(key)
                .map(|toml_value| toml_value.as_bool())
                .flatten();
            
            if bool_opt.is_none() { print_missing_value(key) };
            bool_opt
        }

        // Will always return a positive integer. If the number is negative, will return None
        fn get_positive_integer_value(toml_table: &TomlMap, key: &str) -> Option<i64> {
            let value = toml_table.get(key)
                .map(|toml_value| toml_value.as_integer())
                .flatten();

            match value {
                None => {
                    print_missing_value(key);
                    None
                },
                Some(x) if x < 0 => {
                    print_invalid_value(key, "Number must be greater than 0");
                    None
                },
                Some(x) => Some(x)
            }
        }

        fn get_string_value(toml_table: &TomlMap, key: &str) -> Option<String> {
            let string_opt = toml_table.get(key)
                .map(|toml_value| toml_value.as_str())
                .flatten()
                .map(|str_val| String::from(str_val));

            if string_opt.is_none() { print_missing_value(key) };
            string_opt
        }

        fn get_array_value(toml_table: &TomlMap, key: &str) -> Option<Vec<String>> {
            let vec_opt = toml_table.get(key)
                .map(|toml_value| toml_value.as_array())
                .flatten()
                .map(|strings_vec|{
                    strings_vec.iter()
                        .flat_map(|value| value.as_str())
                        .map(|s|String::from(s))
                        .collect::<Vec<_>>()
                });

            if vec_opt.is_none() { print_missing_value(key) };
            vec_opt
        }

        fn get_dict_value(toml_table: &TomlMap, key: &str) -> Option<HashMap<String, String>> {
            let dict_opt = toml_table.get(key)
                .map(|toml_dict|{ toml_dict.as_table()})
                .flatten()
                .map(|custom_devices| {
                    custom_devices
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

            if dict_opt.is_none() { print_missing_value(key) };
            dict_opt
        }

        fn get_paths(path_strs: Vec<String>) -> Vec<PathBuf> {
            path_strs
                .iter()
                .map(|s|PathBuf::from(s))
                .collect::<Vec<_>>()
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
                                            if let Some(source_paths) = get_array_value(&folders, "source_dirs") {
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
                                            if let Some(source_subdir) = get_string_value(&folders, "source_subdir") {
                                                // get_string_value already filters out empty strings, but just to be safe
                                                if !source_subdir.is_empty() {
                                                    args.append_source_subdir(source_subdir.as_str());
                                                }
                                            }

                                            if let Some(target_dir) = get_string_value(&folders, "target_dir") {
                                                // get_string_value already filters out empty strings, but just to be safe
                                                if !target_dir.is_empty() {
                                                    args.set_target_dir(target_dir);
                                                }
                                            }

                                            if let Some(min_files_per_dir) = get_positive_integer_value(&folders, "min_files_per_dir") {
                                                args.min_files_per_dir = min_files_per_dir;
                                            }

                                            if let Some(oneoffs_dir_name) = get_string_value(&folders, "target_oneoffs_subdir_name") {
                                                // get_string_value already filters out empty strings, but just to be safe
                                                if !oneoffs_dir_name.is_empty() {
                                                    args.oneoffs_dir_name = oneoffs_dir_name;
                                                }
                                            }
                                        } // end if let Some(folders)
                                    } // end Some(folders_opt)
                                    None =>
                                        print_missing_value("folders")
                                } // end config folders

                                /* --- Parse options --- */

                                match toml_content.get("options") {
                                    Some(options_opt) => {
                                        if let Some(options) = options_opt.as_table() {

                                            if let Some(source_recursive) = get_boolean_value(&options, "source_recursive") {
                                                args.source_recursive = source_recursive;
                                            }

                                            if let Some(dry_run) = get_boolean_value(&options, "dry_run") {
                                                args.dry_run = dry_run;
                                            }

                                            if let Some(copy_not_move) = get_boolean_value(&options, "copy_not_move") {
                                                args.copy_not_move = copy_not_move;
                                            }

                                            if let Some(silent) = get_boolean_value(&options, "silent") {
                                                args.silent = silent;
                                            }

                                            // Not exposed in config; use for dev only
                                            // debug_on = true
                                            if let Some(debug_on) = get_boolean_value(&options, "debug_on") {
                                                args.debug = debug_on;
                                            }
                                        }
                                    },
                                    None =>
                                        print_missing_value("options")

                                } // end config options

                                /* --- Parse custom data --- */

                                match toml_content.get("custom") {
                                    Some(custom_data_opt) => {
                                        if let Some(custom_data) = custom_data_opt.as_table() {
                                            if let Some(devices_dict) = get_dict_value(custom_data, "devices") {
                                                args.custom_device_names = devices_dict;
                                            }
                                        } // end if let Some(custom_data)
                                    } // if let Some(custom_data_opt)
                                    None =>
                                        print_missing_value("custom")
                                } // end config custom data
                            },
                            None => {
                                println!("Could not parse TOML into a key-value object");
                            }
                        } // end reading raw toml data
                    }
                    Err(err) => {
                        println!("{}", ColoredString::red(
                            "Could not parse config file, not valid TOML. Continuing with defaults."));
                        eprintln!("{}", err);
                    }
                } // end reading config file contents
            }
            Err(e) => {
                println!("{}", ColoredString::red(format!(
                    "Could not read config file {}. Continuing with defaults.", config_file).as_str()));
                eprintln!("{}", e);
            }
        };
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
            .join("\n> ");

        if valid_paths.is_empty() {
            println!("{}", ColoredString::red(
                format!(
                    "Invalid source folders!\n> {}",
                    list_of_invalid).as_str()));
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        } else {
            println!("{}", ColoredString::orange(
                format!(
                    "Some source folders were invalid and were ignored:\n> {}",
                    list_of_invalid).as_str()));
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