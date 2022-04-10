# imgsorter

Read images and videos in a directory and copy or move them to subdirectories based on file date and device name.

Input:
```
[Source folder]
 └─ DSC_0005.mov
 └─ IMG_0003.jpg
 └─ IMG_0004.jpg
 └─ IMG_0022.jpg
 └─ IMG_0027.jpg
 └─ IMG_1456.jpg
```

Output:
```
[Target folder]
  └─ [imgsorted]
  │    └─ [2014.06.20]
  |    |    └─ IMG_0004.jpg
  |    |    └─ IMG_0022.jpg
  │    └─ [2017.06.22]
  │    |    └─ [Canon EOS 100D]
  |    |    |    └─ IMG_0003.jpg
  |    |    |    └─ IMG_0027.jpg
  │    |    └─ [HUAWEI CAN-L11]
  |    |    |    └─ IMG_1456.jpg
  |    |    └─ DSC_0005.mov
```

## Quick start
1. Edit the `imgsorter.toml` configuration and:
  * add a folder containing some images under `source_dirs`
  * set a destination folder as `target_dir`
2. Run the program
  * Read the confirmation dialog and type `d` to choose option "dry run". This will run a simulation of the copy process with the default options.
  * Inspect the output
3. Repeat step #2 and choose `y` to copy the files
4. Done

Next, you can inspect the options available in `imgsorter.toml` to customize different options (e.g. move files instead of just copying them) and to set other source folders.

## Features
* Move or copy supported files from the source folders to date subfolders in the target folder 
* Date subfolders in the target folder are based on the source files' date in 'YYYY-MM-DD' format 
* The program will ask for confirmation before moving or copying files
* Option to do a 'dry run' which simulates the process without writing any files or directories

## Supported files
* "Fully supported" means "can read EXIF", meaning file will be copied and with accurate info about creation date and device name
* "Partially supported" means "can't read EXIF", meaning file will be copied, but creation date is based on the file metadata and no device name
* "Unsupported" means file will be ignored

* Fully supported files: `jpg`, `png`, `tiff`
* Partially supported image files: `nef`, `crw`
* Partially supported video files: `mp4`, `mov`, `3gp`
* Partially supported audio files: `ogg`, `amr`

## Notes/limitations
* Options can only be set by editing the `imgsorter.toml` configuration file
* File date for supported images is based on the EXIF 'DateTimeOriginal' or 'DateTime' properties
* File date for other files is based on the "modified date" file property
* Device name for supported images is based on the EXIF 'Model' property
* Target directory for supported images is a subdirectory inside the date directory named after the device name (based on EXIF)
* Target directory for other files is inside the root of the date file
* Will **not** overwrite target files if they exist. There's no option currently to toggle this behaviour
* Multiple runs on different source dirs with the **same** target dir may result in mixed images from several devices placed in the same folder
* Unsupported files are ignored and skipped when copying or moving

## FAQ
### The width of the output is too big
Yes. The width of the printed messages for dry runs is based on the maximum length of the source paths
to align everything prettily. If the printed messages are too big for your screen, you can disable
this feature by setting the `align_file_output` config option to false. It won't look as nice, but 
most of the lines should fit in your screen (unless the sources paths are very long, in which case you
could just copy them one at a time - this will trim the path to just the source file name).