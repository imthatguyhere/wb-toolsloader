use config::Config;
use std::collections::HashMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Package {
    id: String,
    name: String,
    description: String,
    version_url: String,
    filelist_url: String,
    repo_url: String,
    url: String,
}

fn main() {
    let exe_path = std::env::current_exe().expect("Failed to get executable path");
    let config_dir = exe_path.parent().expect("Failed to get executable directory");
    let config_path = config_dir.join("Config.toml");
    
    println!("Looking for config at: {:?}", config_path);

    let settings = Config::builder()
        .add_source(config::File::with_name("Config").required(false))
        .add_source(config::File::with_name(config_path.to_str().unwrap()).required(false))
        .build()
        .unwrap();

    // Get the packages table
    let packages: HashMap<String, Package> = settings.get("packages").unwrap_or_default();
    println!("Packages: {:#?}", packages);
}