use std::fs::File;
use std::{fs};
use std::path::Path;
use std::io::{Error, ErrorKind, Result, Write};

extern crate termion;
use termion::{color};

pub fn setup_system() -> Result<()> {
    let setup_dirs_complete = setup_directories_structure();
    if setup_dirs_complete {
        Ok(())
    }else{
        Err(Error::new(ErrorKind::Other, "Checks for directory structure failed"))
    }
}

fn setup_directories_structure() -> bool{
    println!("{}- Initializing directories", color::Fg(color::Yellow));
    // Root directory
    if !graceful_mkdir("data") {return false;}

    // Cache directories
    if !graceful_mkdir("data/cache") {return false;}
    if !graceful_mkdir("data/cache/media") {return false;}
    if !graceful_mkdir("data/cache/meta") {return false;}

    true
}

pub fn graceful_mkdir(dir_path: &str) -> bool {
    let current_path = std::env::current_dir().unwrap();
    let current_path = current_path.as_path().display();
    let path = Path::new(dir_path);
    // Get the metadata attributes of a file/dir and check if it exists or something is wrong
    match fs::metadata(path) {
        Ok(attributes) => {
            if attributes.is_dir() {
                if attributes.permissions().readonly() {
                    println!("{}\t{}/{}: Is not writable{}", color::Fg(color::Red), current_path, path.display(), color::Fg(color::Reset));
                    return false;
                }
                println!("{}\t{}/{}: OK{}", color::Fg(color::Green), current_path, path.display(), color::Fg(color::Reset));
                true
            }
            else {
                println!("{}\t{}/{}: Is not a directory{}", color::Fg(color::Red), current_path,path.display(), color::Fg(color::Reset));
                false
            }
        },
        Err(error) => {
            // Get the error kind to compare later
            match error.kind() {
                ErrorKind::NotFound => {
                    // The dir not exists, create dir
                    let create_result = fs::create_dir(path);
                    match create_result {
                        Ok(_) => {
                            println!("{}\t{}/{}: Created{}", color::Fg(color::Cyan), current_path,path.display(), color::Fg(color::Reset));
                            true
                        },
                        Err(create_error) =>  {
                            println!("{}\t{}/{}: {}{}", color::Fg(color::Red), current_path,path.display(), create_error, color::Fg(color::Reset));
                            false
                        }
                    }
                },
                _ => {
                    println!("{}\tError not managed {}{}", color::Fg(color::Red), error, color::Fg(color::Reset));
                    false
                }
            }
        }
    }
}

pub fn _write_json(file_path: &str, content: String) -> Result<()>
{
    // Current dir to display in log
    let current_path = std::env::current_dir().unwrap();
    let current_path = current_path.as_path().display();

    // Create the path
    let path = Path::new(file_path);
    
    // Create the file, truncates if it exist
    match File::create(path) {
        Ok(mut file) => {
            // Try to write the content
            match file.write_all(content.as_bytes()) {
                Ok(_) => {
                    // Content written
                    //println!("{} File Writed: {}/{}", color::Fg(color::Cyan), current_path, path.display());
                    Ok(())
                },
                Err(error) => {
                    // Pretty error, Cannot be written
                    Err(Error::new(
                        error.kind(), 
                        format!("{}File cannot be written: {}/{}", color::Fg(color::Red), current_path, path.display())
                    ))
                }
            }
        },
        Err(error) => {
            // Pretty error, Cannot be created
            Err(Error::new(
                error.kind(), 
                format!("{}File: {}/{} Cannot be created {:?}", color::Fg(color::Red), current_path, path.display(), error)
            ))
        }
    }
}

#[allow(dead_code)]
pub fn move_file(origin_path: &str, target_path: &str) -> Result<()> {
    match fs::copy(origin_path, target_path) {
        Ok(_) => {
            match fs::remove_file(origin_path) {
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            }
        },
        Err(error) => Err(error),
    }
}