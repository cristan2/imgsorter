use std::path::Path;
use std::{fs, env};
use std::error::Error;
use std::time::UNIX_EPOCH;
use chrono::{DateTime, Utc};
use std::fs::{ReadDir, DirEntry, DirBuilder};
use std::collections::HashMap;

// arg for paths
// arg for min count for move
// arg flag for logfile
// copy/move file only if it's file
// subdirs by device name
// img count in dir name
// change copy to rename
// cleanup
// undo ?

fn main() {

    let cwd = env::current_dir().unwrap();

    println!("current working directory = {}", cwd.display());

    // let base_path = Path::new(".")
    let base_path = cwd
        .join("test_pics");

    let file_list = fs::read_dir(&base_path).unwrap();

    let mut new_dir_tree: HashMap<String, Vec<DirEntry>> = HashMap::new();

    // iterate files, read modified date and create dirs
    for entry in file_list {
        // dbg!(entry);
        let file = entry.unwrap();

        let formatted_time = get_modified_time(&file);

        let sub_dir = base_path.join(&formatted_time);

        let dir_created = DirBuilder::new()
            .recursive(true) // create subdirs if necessary; don't return Err if file exists
            .create(&sub_dir);
        // dbg!(dir_created);

        let new_file_path = sub_dir.join(&file.file_name());


        match fs::copy(&file.path(), &new_file_path) {
            Ok(copy_res) =>
                println!("{} -> {} ... ok",
                         // &file.file_name().to_str().unwrap(),
                         &file.path().strip_prefix(&cwd).unwrap().display(),
                         // &new_file_path.to_str().unwrap()),
                         &new_file_path.strip_prefix(&cwd).unwrap().display()),
            Err(err) =>
                println!("{:?}: ERROR {:?}", &file.file_name(), err)
        };

        let sub_dir = new_dir_tree.entry(formatted_time).or_insert(Vec::new());
        sub_dir.push(file);
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

fn get_modified_time(file: &DirEntry) -> String {
    let metadata = file.metadata().unwrap();
    let created = metadata.modified().unwrap();
    let datetime: DateTime<Utc> = created.into(); // 2021-06-05T16:26:22.756168300Z
    datetime.format("%Y.%m.%d").to_string()
}