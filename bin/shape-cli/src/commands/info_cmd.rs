use crate::registry_client::RegistryClient;

fn format_downloads(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

pub async fn run_info(name: String) {
    let client = RegistryClient::new(None);

    let info = match client.get_info(&name).await {
        Ok(i) => i,
        Err(e) => {
            if e.contains("404") || e.contains("not found") {
                eprintln!("Package '{}' not found", name);
            } else {
                eprintln!("Error: {}", e);
            }
            std::process::exit(1);
        }
    };

    println!("Package: {}", info.name);
    if let Some(desc) = &info.description {
        println!("Description: {}", desc);
    }
    if let Some(license) = &info.license {
        println!("License: {}", license);
    }
    if let Some(repo) = &info.repository {
        println!("Repository: {}", repo);
    }
    if let Some(homepage) = &info.homepage {
        println!("Homepage: {}", homepage);
    }
    println!("Downloads: {}", format_downloads(info.downloads));
    if !info.owners.is_empty() {
        println!("Owners: {}", info.owners.join(", "));
    }

    // Show latest version permissions
    if let Some(latest) = info.versions.first() {
        if !latest.required_permissions.is_empty() {
            println!();
            println!("Required Permissions (latest):");
            for perm in &latest.required_permissions {
                println!("  - {}", perm);
            }
        }

        if latest.has_native_deps {
            println!();
            println!("Native Dependencies (latest):");
            if !latest.native_platforms.is_empty() {
                println!("  Platforms: {}", latest.native_platforms.join(", "));
            }
        }
    }

    // Version list
    if !info.versions.is_empty() {
        println!();
        println!("Versions:");
        for v in &info.versions {
            let yanked = if v.yanked { "  [yanked]" } else { "" };
            let sig = match &v.author_key {
                Some(key) => format!("  signed by {}", key),
                None => String::new(),
            };
            println!(
                "  {:<12} {:<12} {} downloads{}{}",
                v.version,
                v.published_at,
                format_downloads(v.downloads),
                sig,
                yanked,
            );
        }
    }
}
