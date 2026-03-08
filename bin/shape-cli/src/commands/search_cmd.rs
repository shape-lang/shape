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

pub async fn run_search(query: String) {
    let client = RegistryClient::new(None);

    let results = match client.search(&query).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    if results.is_empty() {
        println!("No packages found matching '{}'", query);
        return;
    }

    // Calculate column widths
    let name_width = results
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let version_width = results
        .iter()
        .map(|r| r.version.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let dl_width = results
        .iter()
        .map(|r| format_downloads(r.downloads).len())
        .max()
        .unwrap_or(9)
        .max(9);

    // Header
    println!(
        "{:<name_w$}  {:<ver_w$}  {:>dl_w$}  DESCRIPTION",
        "NAME",
        "VERSION",
        "DOWNLOADS",
        name_w = name_width,
        ver_w = version_width,
        dl_w = dl_width,
    );

    // Rows
    for pkg in &results {
        let desc = pkg.description.as_deref().unwrap_or("");
        println!(
            "{:<name_w$}  {:<ver_w$}  {:>dl_w$}  {}",
            pkg.name,
            pkg.version,
            format_downloads(pkg.downloads),
            desc,
            name_w = name_width,
            ver_w = version_width,
            dl_w = dl_width,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_downloads() {
        assert_eq!(format_downloads(0), "0");
        assert_eq!(format_downloads(999), "999");
        assert_eq!(format_downloads(1_000), "1,000");
        assert_eq!(format_downloads(1_234), "1,234");
        assert_eq!(format_downloads(1_234_567), "1,234,567");
    }
}
