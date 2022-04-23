# imgsorter

Read image, video and audio files in a folder and copy or move them to subfolders based on the file date and possibly the device name.

Input:
```
[Source folder]
 ├─ DSC_0005.mov
 ├─ IMG_0003.jpg
 ├─ IMG_0004.jpg
 ├─ IMG_0022.jpg
 ├─ IMG_0027.jpg
 └─ IMG_1456.jpg
```

Output:
```
[Target folder]
 └─ [imgsorted]
     ├─ [2014.06.20]
     │   ├─ IMG_0004.jpg
     │   └─ IMG_0022.jpg
     └─ [2017.06.22]
         ├─ [Canon EOS 100D]
         |   ├─ IMG_0003.jpg
         |   └─ IMG_0027.jpg
         ├─ [Huawei CAN-L11]
         |   └─ IMG_1456.jpg
         └─ DSC_0005.mov
```

## Quick start
1. Edit the `imgsorter.toml` configuration and:
  * add a folder containing some images under `source_dirs`
  * set a destination folder as `target_dir`
2. Run the program
  * Read the confirmation dialog and type `d` to choose option "dry run". This will run a simulation of the copy process with the default options.
  * Inspect the output
3. Repeat step #2 and type `y` when prompted to copy the files
4. Done

Next, you can inspect the options available in `imgsorter.toml` to customize different options (e.g. move files instead of just copying them) and to set other source folders.

## Features
* Move or copy supported files from the source folders to date subfolders in the target folder 
* Date subfolders in the target folder are based on the source files' date in 'YYYY-MM-DD' format 
* The program will ask for confirmation before moving or copying files
* Option to do a 'dry run' which simulates the process without writing any files or folders

## Supported files
* _"Fully supported"_ means "can read EXIF", meaning file will be copied with accurate info about creation date and device name
* _"Partially supported"_ means "cannot read EXIF", meaning file will be copied, but creation date is based on the file metadata and no device name
* _"Unsupported"_ means file will be ignored


* Fully supported file formats: `jpg`, `png`, `tiff`, `heic` or `heif`, `webp`, `avif`
* Partially supported image files: `nef`, `nrw`, `crw`
* Partially supported video files: `mp4`, `mov`, `3gp`, `avi`
* Partially supported audio files: `ogg`, `amr`, "m4a"

## Notes/limitations
* Options can only be set by editing the [imgsorter.toml](imgsorter.toml) configuration file
* File date for supported images is based on the EXIF 'DateTimeOriginal' or 'DateTime' properties
* File date for other files is based on the "modified date" file property
* Device names are created based on the EXIF `Make` and `Model` properties, where available
* Target folder for supported images is a subfolder (inside the date folder) named after the device name (based on EXIF)
* Target folder for other files is inside the root of the date file
* Will **not** overwrite target files if they exist. There's no option currently to toggle this behaviour
* Multiple runs on different source dirs with the **same** target dir may result in mixed images from several devices placed in the same folder
* Unsupported files are ignored and skipped when copying or moving

## Usage
### Example workflow
* Start by identifying the folder(s) containing the files to be sorted
* Optionally, pre-filter them manually by selecting all files you don't want to sort by date and device and move them to a different folder (e.g. keep all cat pictures in a single folder, regardless of their date)
* Edit the source paths in the configuration file
* Start the program; do a dry run first by choosing `d` when prompted
* Inspect the result and statistics and go over the list of files to see:
  * how many and which files will be skipped
  * what file types will be skipped based on extension
  * how many folders will be created
  * how many files will be placed in the Miscellaneous folder
  * if there are many folders with too few files which might be better bunched together in the Miscellaneous folder instead
* Make the necessary adjustments in the configuration file, like for example:
  * set different limits for minimum files per directory
  * customize device names
  * include additional file extensions to be processed
* Do a second dry run and confirm changes
* Run the program again and choose 'y' to copy the files
* Inspect the result and delete the source files manually (unless you enabled the "copy_not_move" configuration option before)
* Repeat the process with different source folder(s)

**Note**: the sorting works best if you set all source folders in a single run. If you do several sorting operations with the same target folder, it's possible not all files will be sorted correctly. For instance, images from different devices might be placed together in a single date folder instead of being separated by device. To correct this, once you are finished sorting all your source folders to the same target folder, you can do one last sort operation and set the previous target folder as the source. This way, *all* files will be read and sorted correctly. 

## FAQ
### Is there no other way to configure this program other than editing a configuration file?
Currently, no, there isn't. Most settings have sensible defaults, but you'll have to at least configure the source folders. You can then read the description for each setting in the [configuration file](imgsorter.toml) to get a sense of their purpose and what other configuration options you have available.

### The program just copies files, how do I **move** them?
Edit the configuration file and set `copy_not_move` to `true`.

### I'm getting a lot of folders with only one or two images
Since the sorting is primarily done based on the image date, this will happen when there are very few images taken on any given day ("one-off" images). In these cases, the program will not create a date folder for them and just move all these files in a single separate folder named `Miscellaneous` (configurable). To control this, the configuration file has the option to set `min_files_per_dir`, which is the minimum number of files required for a target date folder to be created. This doesn't apply if there are images from more than one device - in this case, all required date and device folders will be created even if the total number of files for this date are less than `min_files_per_dir`.

### Some date folders contain images without a device folder
There are two possible scenarios which can lead to this. Some images don't have the required EXIF data to determine the device name and create a folder. In other cases, if all images for a given date are taken with a single device, no separate device folder is created, to avoid having a folder-in-folder situation for no reason. Instead, all files are placed directly in the root of the date folder. To force the program to always create a directory, set the configuration option `always_create_device_subdirs` to `true`.

Note that if this is enabled, partially supported files which don't have a (readable) EXIF device information will be placed in a default folder named "Unknown". This also affects files in date directories which contain a mix of supported and partially supported files - previously, the partially supported files would be placed directly in the root of the date folder, while now an "Unknown" folder will be created for them.

### The width of the output is too big
The width of the printed messages for dry runs is based on the maximum length of the source paths
to align everything prettily. If the printed messages are too big for your window, you can disable
this feature by setting the `align_file_output` config option to `false`. It won't look as nice, but 
most of the lines should fit in your screen (unless the source paths are very long, in which case you
could just copy them folder at a time - for single source paths, the program will trim the path to just the source file name).

### The list of files is too long/uninteresting
The length of the output depends on the number of source files to be processed. For typical operations, this may consist of a long list of files which will be copied without issues, so the output will not provide much useful information. To address this, the configuration file offers a "compact" mode: set the  `min_files_before_compacting_output` to a low number (e.g. 3) and now the output will not print the status for any consecutive files in the same folder with the same status. This applies to dry runs only.

### My source folders contain additional file formats which I want to have sorted
Not all file types are supported by default. If your source folders contain unknown files, their extensions will be listed at the end of a dry run. If you want to include any of these file types, edit the configuration file and add their extension in the appropriate category under `[custom.extensions]`. For example: `image = [ "gif" ]`. These files will then be considered "partially supported", meaning they'll be processed based on their "modified date" metadata only.

### The device names are not very descriptive. What does `SM-A415F` even mean?
Images are sorted into folders based on their date and, if EXIF data is available, the device name which was used to record the image or video. However, this might be different than the name you're expecting. For instance, "SM-A415F" is the model name for "Samsung A41". If you don't like these names, you can set custom names for each model by adding them in the configuration file under `[custom.devices]`. like for example `'SM-A415F'="Maria's phone"`.

### I know what I'm doing, I don't want to bother confirming every operation
Fine, just set the configuration key `silent` to `true` and you're good to go. 

### I know what I'm doing, but the configuration file is too messy
For convenience, there's a second configuration file you can use, `imgsorter_clean.toml`, which contains the same configuration settings as `imgsorter.toml` but without any comments. Just rename this file to `imgsorter.toml` and use it instead (remember to delete or rename the old one first).
