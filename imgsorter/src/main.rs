use std::path::{PathBuf, Path};
use std::{fs, env};
use chrono::{DateTime, Utc};
use std::fs::{DirEntry, DirBuilder, File};
use std::collections::HashMap;
use rexif::{ExifEntry, ExifTag, ExifResult};
use std::io::{Read, Seek, SeekFrom};

const DBG_ON: bool = true;

fn main() {

    let cwd = env::current_dir().unwrap();

    println!("current working directory = {}", cwd.display());

    // TODO 1a: temporary citim din ./test_pics, normal ar trebui sa fie argument sau doar current dir
    // let path_cwd = Path::new(".")
    let cwd_path: PathBuf = cwd.join("test_pics");

    // TODO 2b: filter files, images
    let all_files = fs::read_dir(&cwd_path).unwrap();

    // let img_list = all_files.filter(|f| {
    //     *f.unwrap().file_type()
    // })

    // TODO 3a: add printout no of files
    // iterate files, read modified date and create dirs
    for entry in all_files {

        if DBG_ON {
            /* Print whole entry */
            println!("---------------");
            dbg!(&entry);
            println!("---------------");
        }

        // TODO 5: unwrap here?
        let file: DirEntry = entry.unwrap();

        if DBG_ON {
            /* Print extensions */
            let n_path = &file.path();
            if let Some(n) = n_path.extension() {
                let ext = n.to_str().unwrap_or("");
                println!("Ext: '{}'", ext);
            } else {
                println!("No extension for : '{}'", n_path.to_str().unwrap_or("??"));
            }
        }

        // TODO 5a: parse file to "supported file" struct

        let formatted_time_opt: Option<String> = get_modified_time(&file);
        let device_name_opt: Option<String> = get_device_name(&file);

        formatted_time_opt.map(|date|
            sort_file_to_subdir(file, date, &cwd_path, device_name_opt));
    }
}

fn print_file_list(dir_tree: HashMap<String, Vec<DirEntry>>) {
    dir_tree.iter().for_each(|(dir_name, dir_files)|{
        println!("{} ({})", dir_name, dir_files.len());
        for file in dir_files {
            let filename = &file.file_name();
            println!("| + {}", filename.to_str().unwrap());
        }
        // dbg!(dir_files);
    })
}

/// Move the file to a subdirectory named after the file date
/// Optionally, create additional subdir based on device name
fn sort_file_to_subdir(file: DirEntry, date: String, cwd_path: &PathBuf, device_name_opt: Option<String>) {
    // Create target subdir based on image date
    let mut target_subdir_path = cwd_path
        // TODO 1d: based on target flag to create subdir for sorted files, or sort in-place?
        .join("imgsorted")
        .join(&date);

    // attach device name subdir path
    if let Some(device_name) = device_name_opt {
        // TODO 4a - replace device name with custom name from config
        target_subdir_path.push(&device_name);
    }

    println!("subdir = {:?}, path_cwd = {:?}", &target_subdir_path, cwd_path);

    create_subdir_if_required(&target_subdir_path, cwd_path);

    // attach file path
    // TODO 5: create new path variable?
    target_subdir_path.push(&file.file_name());

    // copy file
    // TODO 6a: move instead of copy
    copy_file_if_not_exists(&file, &target_subdir_path, cwd_path);
}

fn copy_file_if_not_exists(file: &DirEntry, target_subdir: &PathBuf, path_cwd: &PathBuf) {
    let file_copy_status = if target_subdir.exists() {
        "already exists"
    } else {
        match fs::copy(file.path(), target_subdir) {
            Ok(_) =>
                "ok",
            Err(err) =>
                // println!("File copy error: {:?}: ERROR {:?}", &file.file_name(), err)
                // TODO add error info
                "ERROR"
        }
    };

    println!("Copying {} -> {} ... {}",
             // &file.file_name().to_str().unwrap(),
             file.path().strip_prefix(path_cwd).unwrap().display(),
             // &new_file_path.to_str().unwrap()),
             target_subdir.strip_prefix(path_cwd).unwrap().display(),
             file_copy_status);
}

fn create_subdir_if_required(target_subdir: &PathBuf, path_cwd: &PathBuf) {
    if target_subdir.exists() {
        // TODO 2f: handle dir already exists, maybe just log it?
        // println!("target dir exists: {}: {}",
        //          &target_subdir.strip_prefix(&path_cwd).unwrap().display(),
        //          &target_subdir.exists());
        // "already exists"
    } else {
        let subdir_creation = DirBuilder::new()
            // create subdirs if necessary; don't return Err if file exists
            .recursive(true)
            .create(target_subdir);

        match subdir_creation {
            Ok(_) => {
                println!("(Created subdirectory {})",
                         target_subdir.strip_prefix(&path_cwd).unwrap().display());
            },
            Err(e) =>
            // TODO 2f: handle dir creation fail
                println!("Failed to create subdirectory {}: {:?}",
                         target_subdir.strip_prefix(&path_cwd).unwrap().display(),
                         e.kind())
        }
    };
}

fn get_modified_time(file: &DirEntry) -> Option<String> {
    let modified_time = if let Ok(metadata) = file.metadata() {
        match metadata.modified() {
            Ok(created) => {
                let datetime: DateTime<Utc> = created.into(); // 2021-06-05T16:26:22.756168300Z
                Some(datetime.format("%Y.%m.%d").to_string())
            },
            Err(_) =>
                None
        }
    } else {
        None
    };

    modified_time
}

fn get_device_name(file: &DirEntry) -> Option<String> {
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
            println!("Error in {:?}: {}", &file_name, e.to_string());
            None
        }
    }

}

/// Replicate implementation of `rexif::parse_file` and `rexif::read_file`
/// to bypass `rexif::parse_buffer` which prints warnings to stderr
fn read_exif_file<P: AsRef<Path>>(file_name: P) -> ExifResult {
    // let file_name = file_entry.path();
    let mut file = File::open(file_name).unwrap();
    &file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents: Vec<u8> = Vec::new();
    &file.read_to_end(&mut contents);
    let (res, _) = rexif::parse_buffer_quiet(&contents);
    res
}