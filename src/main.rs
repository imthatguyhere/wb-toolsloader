use config::Config;
use std::collections::HashMap;
use std::io::{self, Write};
use serde::Deserialize;
use reqwest::blocking::Client;

#[derive(Debug, Deserialize, Clone)]
struct Package {
    id: String,
    name: String,
    description: String,
    version_url: String,
    filelist_url: String,
    repo_url: String,
}

#[derive(Debug, Deserialize)]
struct Settings {
    archive: HashMap<String, String>,
    packages: HashMap<String, Package>,
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

fn main() {
    let exe_path = std::env::current_exe().expect("Failed to get executable path");
    let config_dir = exe_path.parent().expect("Failed to get executable directory");
    let config_path = config_dir.join("Config.toml");
    
    println!("Looking for config at: {:?}", config_path);

    let settings: Settings = Config::builder()
        .add_source(config::File::with_name("Config").required(false))
        .add_source(config::File::with_name(config_path.to_str().unwrap()).required(false))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap();

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
        None // All packages = None
    } else {
        // Parse and validate number, handling cases like "1." or "1.0"
        let num = input.split('.').next().unwrap_or("").parse::<usize>();
        match num {
            Ok(num) if num > 0 && num <= package_vec.len() => Some(num - 1),
            _ => {
                println!("Invalid selection");
                return;
            }
        }
    };

    println!("\n{}:", if selected_index.is_some() { "Package" } else { "Packages" });
    for (i, (_, package)) in package_vec.iter().enumerate() {
        if let Some(idx) = selected_index {
            if i != idx {
                continue;
            }
        }
        
        // Print version and check availability
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

        // Only proceed with file listing if version was available
        if is_available {
            // Get and print files
            println!("\n{} ({}) files:", package.name, package.id);
            match get_package_files(package) {
                Ok(files) => {
                    let repo_url = if package.repo_url.ends_with('/') {
                        package.repo_url.clone()
                    } else {
                        format!("{}/", package.repo_url)
                    };
                    
                    for file in files {
                        println!("{}{}", repo_url, file);
                    }
                },
                Err(e) => println!("Error fetching file list:\n {}", e),
            }
        }
        
        println!(); // Add a blank line between packages
        
        if selected_index.is_some() {
            break;
        }
    }
}