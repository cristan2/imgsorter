use std::path::PathBuf;
use std::{fs, env};
use chrono::{DateTime, Utc};
use std::fs::{DirEntry, DirBuilder};
// use std::collections::HashMap;
use rexif::{ExifEntry, ExifTag};

// [ ] arg for paths
// [ ] arg for min count for move
// [ ] arg flag for logfile
// [ ] copy/move file only if it's file
// [x] subdirs by device name
// [ ] img count in dir name
// [ ] change copy to rename
// [ ] cleanup
// [ ] undo ?

fn main() {

    let cwd = env::current_dir().unwrap();

    println!("current working directory = {}", cwd.display());

    // TODO temp, ar trebui sa fie argument sau doar current dir
    // let path_cwd = Path::new(".")
    let path_cwd: PathBuf = cwd.join("test_pics");

    // TODO filter files, images
    let file_list = fs::read_dir(&path_cwd).unwrap();

    // let mut new_dir_tree: HashMap<String, Vec<DirEntry>> = HashMap::new();

    // TODO add printout no of files
    // iterate files, read modified date and create dirs
    for entry in file_list {
        // dbg!(entry);
        let file = entry.unwrap(); // TODO unwrap?

        let formatted_time: Option<String> = get_modified_time(&file);
        let device_name: Option<String> = get_device_name(&file);

        // create image date subdir path
        if let Some(date) = formatted_time {
            let mut target_subdir = path_cwd.join(&date);

            // attach device name subdir path
            if let Some(device) = device_name {
                target_subdir.push(&device);
            }

            create_subdir_if_required(&target_subdir, &path_cwd);

            // attach file path
            // TODO create new path variable?
            target_subdir.push(&file.file_name());

            // copy file
            copy_file_if_not_exists(&file, &target_subdir, &path_cwd);
        }

        /*let sub_dir = new_dir_tree.entry(formatted_time).or_insert(Vec::new());
        sub_dir.push(file);*/
    }


/*    new_dir_tree.iter().for_each(|(dir_name, dir_files)|{
        println!("{} ({})", dir_name, dir_files.len());
        for file in dir_files {
            let filename = &file.file_name();
            println!("| + {}", filename.to_str().unwrap());
        }
        // dbg!(dir_files);
    })*/
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
        // TODO maybe log it exists?
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
            // TODO handle dir creation fail
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

    // TODO parse_file produces warnings to stderr
    match rexif::parse_file(&file_name) {

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