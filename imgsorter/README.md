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
[Source folder]
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

## Features
* Move or copy files based on input arguments
* Target subdirectories are named after the files' input date in 'YYYY-MM-DD' format 
* The program will ask for confirmation before moving or copying files
* Option to do a 'dry run' which simulates the process without writing any files or directories

## Supported files
* Images: jpg, png, tiff, nef, crw
* Video: mp4, mov, 3gp
* EXIF data: jpg, png, tiff only

## Notes/limitations:
* Options can not be set interactively at the moment.
* File date is based on "modified date" file property
* Device name is based on EXIF data for image files;
* If EXIF is not present or the file or device is not supported, target is the root of the date file
* No device name can be determined for video files at the moment
* Only the files in the root of the source directory are read at the moment. Any subdirectories will be ignored.
* Unsupported files are ignored