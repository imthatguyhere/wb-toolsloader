use config::Config;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;
use serde::Deserialize;
use reqwest::blocking::Client;
use regex::Regex;

#[derive(Debug, Deserialize, Clone)]
struct Package {
    id: String,
    name: String,
    description: String,
    version_url: String,
    filelist_url: String,
    repo_url: String,
    output_path: String,
}

#[derive(Debug, Deserialize)]
struct Settings {
    archive: HashMap<String, String>,
    packages: HashMap<String, Package>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    date: String,
    iteration: i32,
}

impl Version {
    fn parse(version_str: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let parts: Vec<&str> = version_str.trim().split("--").collect();
        if parts.len() != 2 {
            return Err("Invalid version format. Expected YYYY-MM-DD--N".into());
        }

        let date = parts[0].to_string();
        let iteration = parts[1].parse::<i32>()?;

        Ok(Version { date, iteration })
    }

    fn verdate_to_string(&self) -> String {
        format!("{}--{}", self.date, self.iteration)
    }
}

fn get_current_version(output_dir: &Path) -> Result<Option<Version>, Box<dyn std::error::Error>> {
    let version_file = output_dir.join("version.txt");
    if !version_file.exists() {
        return Ok(None);
    }
    
    let content = fs::read_to_string(version_file)?;
    Ok(Some(Version::parse(&content)?))
}

fn should_update_package(current: Option<&Version>, new: &Version) -> Result<bool, Box<dyn std::error::Error>> {
    match current {
        None => Ok(true),
        Some(current) => {
            if current == new {
                print!("Package version is the same. Reload anyway? (Y/N) [N]: ");
                io::stdout().flush()?;
                let mut choice = String::new();
                io::stdin().read_line(&mut choice)?;
                Ok(choice.trim().eq_ignore_ascii_case("Y"))
            } else if current > new {
                println!("Local version ({}) is newer than repository version ({})", 
                    current.verdate_to_string(), new.verdate_to_string());
                print!("Download older version from repository? (Y/N) [N]: ");
                io::stdout().flush()?;
                let mut choice = String::new();
                io::stdin().read_line(&mut choice)?;
                Ok(choice.trim().eq_ignore_ascii_case("Y"))
            } else {
                Ok(true) // current < new, should update
            }
        }
    }
}

fn get_version(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new();
    let response = client.get(url).send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("Version cannot be retrieved: 404 Not Found".into());
    }
    Ok(response.text()?.trim().to_string())
}

fn get_package_version_string(package: &Package) -> Result<String, Box<dyn std::error::Error>> {
    let version = get_version(&package.version_url)?;
    Ok(format!("{}: {}", package.name, version))
}

fn get_package_files(package: &Package) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = Client::new();
    let response = client.get(&package.filelist_url).send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("File list cannot be retrieved: 404 Not Found".into());
    }
    
    let content = response.text()?;
    let files: Vec<String> = content
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    
    Ok(files)
}

fn transform_filename(filename: &str) -> Option<String> {
    let re = Regex::new(r"--n(\d+)\.globby$").ok()?;
    let caps = re.captures(filename)?;
    let number = caps.get(1)?.as_str().parse::<i32>().ok()?;
    let new_suffix = format!(".7z.{:03}", number);
    Some(re.replace(filename, new_suffix).to_string())
}

fn get_base_name(filename: &str) -> Option<String> {
    filename.split(".7z.").next().map(|s| s.to_string())
}

fn download_file(url: &str, target_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    //==- Create parent directories if they don't exist
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let client = Client::new();
    let response = client.get(url).send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("File cannot be downloaded: 404 Not Found".into());
    }

    let mut file = fs::File::create(target_path)?;
    io::copy(&mut response.bytes()?.as_ref(), &mut file)?;
    Ok(())
}

fn handle_output_dir(output_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if output_dir.exists() {
        print!("(O)verwrite or (D)elete output folder? [O]: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim().to_uppercase();
        
        if choice == "D" {
            fs::remove_dir_all(output_dir)?;
            fs::create_dir_all(output_dir)?;
            println!("Deleted and recreated output folder");
        } else {
            // Default to overwrite (empty input or "O")
            println!("Will overwrite existing files");
        }
    } else {
        fs::create_dir_all(output_dir)?;
    }
    Ok(())
}

fn extract_archives(nanazip_path: &Path, package_dir: &Path, output_dir: &Path, password: &str) -> Result<(), Box<dyn std::error::Error>> {
    //==- Handle output directory first
    handle_output_dir(output_dir)?;

    let archives: Vec<_> = fs::read_dir(package_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.starts_with("001"))
                .unwrap_or(false)
        })
        .collect();
    for archive in archives {
        let archive_path = archive.path();
        if let Some(base_name) = get_base_name(archive_path.file_name().unwrap().to_str().unwrap()) {
            let extract_dir = package_dir.join(&base_name);
            fs::create_dir_all(&extract_dir)?;

            let mut cmd = Command::new(nanazip_path);
            cmd.current_dir(package_dir)
               .arg("x")
               .arg("-y") //==- Force yes on all queries
               .arg(&archive_path)
               .arg(format!("-o{}", extract_dir.display()));

            if !password.is_empty() {
                cmd.arg(format!("-p{}", password));
            }

            match cmd.output() {
                Ok(output) => {
                    if !output.status.success() {
                        let error_msg = String::from_utf8_lossy(&output.stderr);
                        if error_msg.contains("Wrong password?") {
                            return Err("Wrong password".into());
                        }
                        return Err(format!(
                            "Failed to extract {}: {}", 
                            archive_path.display(),
                            error_msg
                        ).into());
                    }
                    println!("Extracted {} to {}", archive_path.display(), extract_dir.display());

                    //==- Move extracted files to output directory
                    fs::create_dir_all(output_dir)?;
                    for entry in fs::read_dir(&extract_dir)? {
                        let entry = entry?;
                        let target_path = output_dir.join(entry.file_name());
                        fs::rename(entry.path(), target_path)?;
                    }
                    println!("Moved files to {}", output_dir.display());

                    //==- Clean up extraction directory
                    fs::remove_dir_all(&extract_dir)?;
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("!==-- The config for NanaZip's location is incorrect. NanaZip executable not found. --==!");
                    return Err(Box::new(e));
                },
                Err(e) => return Err(Box::new(e)),
            }
        }
    }
    Ok(())
}

fn save_version_file(version: &Version, output_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let version_file = output_dir.join("version.txt");
    let version_str = format!("{}--{}", version.date, version.iteration);
    fs::write(version_file, version_str)?;
    Ok(())
}

fn cleanup_package_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

fn main() {
    let exe_path = std::env::current_exe().expect("Failed to get executable path");
    let config_dir = exe_path.parent().expect("Failed to get executable directory");
    let config_path = config_dir.join("Config.toml");
    let dl_dir = config_dir.join("dl");
    
    println!("Looking for config at: {:?}", config_path);

    let settings: Settings = Config::builder()
        .add_source(config::File::with_name("Config").required(false))
        .add_source(config::File::with_name(config_path.to_str().unwrap()).required(false))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap();

    //==- Get NanaZip path from config and resolve it relative to the executable directory
    let nanazip_relative_path = settings.archive.get("nanazip_exe")
        .expect("nanazip_exe not found in config");
    let nanazip_path = config_dir.join(nanazip_relative_path);

    //==- Convert the packages to a sorted vec
    let mut package_vec: Vec<(&String, &Package)> = settings.packages.iter().collect();
    package_vec.sort_by(|a, b| a.1.name.cmp(&b.1.name));

    if package_vec.is_empty() {
        println!("No packages found in config!");
        return;
    }

    //==- Display numbered list
    println!("\nAvailable packages:");
    println!("A. All packages");
    for (i, (_, package)) in package_vec.iter().enumerate() {
        println!("{}. {}", i + 1, package.name);
    }

    //==- Get user input from the console
    print!("\nSelect a package number (or A for all): ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    
    let input = input.trim();
    
    //==- Parse selection
    let selected_index = if input.eq_ignore_ascii_case("a") || input.eq_ignore_ascii_case("all") {
        None //==- All packages = None
    } else {
        //==- Parse and validate number, handling cases like "1." or "1.0"
        let num = input.split('.').next().unwrap_or("").parse::<usize>();
        match num {
            Ok(num) if num > 0 && num <= package_vec.len() => Some(num - 1),
            _ => {
                println!("Invalid selection");
                return;
            }
        }
    };

    let mut last_password = String::new();

    println!("\n{}:", if selected_index.is_some() { "Package" } else { "Packages" });
    for (i, (_, package)) in package_vec.iter().enumerate() {
        if let Some(idx) = selected_index {
            if i != idx {
                continue;
            }
        }
        
        //==- Print version and check availability
        let is_available = match get_package_version_string(package) {
            Ok(version_string) => {
                println!("{}", version_string);
                true
            },
            Err(e) => {
                println!("{} is not available:\n {}", package.name, e);
                false
            }
        };

        //==- Only proceed with file listing if version was available
        if is_available {
            //==- Get and print files
            println!("\n{} ({}) files:", package.name, package.id);
            match get_package_files(package) {
                Ok(files) => {
                    let repo_url = if package.repo_url.ends_with('/') {
                        package.repo_url.clone()
                    } else {
                        format!("{}\\", package.repo_url)
                    };
                    
                    let package_dl_dir = dl_dir.join(&package.id);
                    let package_output_dir = config_dir.join(&package.output_path);

                    //==- Get and check version before downloading files
                    let version = match get_version(&package.version_url) {
                        Ok(v) => match Version::parse(&v) {
                            Ok(parsed) => parsed,
                            Err(e) => {
                                println!("Failed to parse version: {}", e);
                                continue;
                            }
                        },
                        Err(e) => {
                            println!("Failed to get version: {}", e);
                            continue;
                        }
                    };

                    //==- Check current version and prompt if needed
                    let current_version = match get_current_version(&package_output_dir) {
                        Ok(v) => v,
                        Err(e) => {
                            println!("Failed to read current version: {}", e);
                            None
                        }
                    };

                    match should_update_package(current_version.as_ref(), &version) {
                        Ok(true) => {
                            if let Some(current) = &current_version {
                                if current > &version {
                                    println!("Downgrading to version: {}", version.verdate_to_string());
                                } else {
                                    println!("Updating to version: {}", version.verdate_to_string());
                                }
                            } else {
                                println!("Installing version: {}", version.verdate_to_string());
                            }
                        },
                        Ok(false) => {
                            println!("Skipping package update");
                            continue;
                        },
                        Err(e) => {
                            println!("Error checking version: {}", e);
                            continue;
                        }
                    };
                    
                    for file in files {
                        let file_url = format!("{}{}", repo_url, file);
                        println!("{}", file_url);
                        
                        //==- Transform filename and download
                        if let Some(new_filename) = transform_filename(&file) {
                            let target_path = package_dl_dir.join(&new_filename);
                            match download_file(&file_url, &target_path) {
                                Ok(_) => println!("Downloaded as: {}", new_filename),
                                Err(e) => println!("Error downloading {}: {}", file, e),
                            }
                        } else {
                            println!("Error: Could not transform filename: {}", file);
                        }
                    }

                    //==- Prompt for password and handle retries
                    let mut retry_mode = false;
                    loop {
                        let prompt = if retry_mode {
                            "Enter password for extraction (press Enter [on a blank entry] to skip this package)"
                            } else if last_password.is_empty() {
                                "Enter password for extraction"
                            } else {
                                "Enter password for extraction (press Enter [on a blank entry] to use previous password)"
                            };
                        
                        print!("\n{}: ", prompt);
                        io::stdout().flush().unwrap();
                        let mut password = String::new();
                        io::stdin().read_line(&mut password).unwrap();
                        let password = password.trim();
                        
                        let current_password = if !retry_mode && password.is_empty() {
                            &last_password
                        } else if password.is_empty() {
                            println!("Skipping package due to empty password");
                            break;
                        } else {
                            password
                        };

                        //==- Extract archives
                        match extract_archives(&nanazip_path, &package_dl_dir, &package_output_dir, current_password) {
                            Ok(_) => {
                                println!("Successfully extracted archives");
                                if !password.is_empty() {
                                    last_password = password.to_string(); //==- Only save non-empty passwords if successful
                                }
                                break; //==- Exit password retry loop on success
                            },
                            Err(e) => {
                                println!("Error during extraction: {}", e);
                                retry_mode = true;
                                continue;
                            }
                        }
                    }

                    //==- Save version file after all archives are successfully extracted
                    if let Err(e) = save_version_file(&version, &package_output_dir) {
                        println!("Warning: Failed to save version file: {}", e);
                    }

                    //==- Clean up downloaded files
                    if let Err(e) = cleanup_package_dir(&package_dl_dir) {
                        println!("Error cleaning up package directory: {}", e);
                    }
                },
                Err(e) => println!("Error fetching file list:\n {}", e),
            }
        }
        
        println!(); //==- Add a blank line between packages
    }

    //==- Clean up main download directory
    if let Err(e) = cleanup_package_dir(&dl_dir) {
        println!("Error cleaning up download directory: {}", e);
    }
}