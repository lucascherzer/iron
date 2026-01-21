use anyhow::{Result, anyhow};
use iroh::SecretKey;
use iron::keys;
use std::fs;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub fn run(
    prefix: String,
    threads: Option<usize>,
    max_attempts: Option<u64>,
    save: bool,
    output: Option<String>,
    quiet: bool,
) -> Result<()> {
    // Validate prefix
    validate_prefix(&prefix)?;

    let prefix_lower = prefix.to_lowercase();
    let num_threads = threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });

    if !quiet {
        println!(
            "\nSearching for vanity address with prefix \"{}\"...",
            prefix
        );
        println!("Threads: {}", num_threads);
        print_difficulty_estimate(&prefix_lower);
        println!();
    }

    // Shared state
    let found = Arc::new(AtomicBool::new(false));
    let total_attempts = Arc::new(AtomicU64::new(0));
    let start_time = Instant::now();

    // Spawn worker threads
    let mut handles = vec![];
    let (tx, rx) = std::sync::mpsc::channel();

    for _thread_id in 0..num_threads {
        let prefix = prefix_lower.clone();
        let found = Arc::clone(&found);
        let total_attempts = Arc::clone(&total_attempts);
        let tx = tx.clone();

        let handle = std::thread::spawn(move || {
            let mut local_attempts = 0u64;
            let mut rng = rand::rng();

            loop {
                // Check if another thread found a match
                if found.load(Ordering::Relaxed) {
                    break;
                }

                // Generate a key
                let secret_key = SecretKey::generate(&mut rng);
                let endpoint_id = secret_key.public();
                let base32_id = data_encoding::BASE32_NOPAD
                    .encode(endpoint_id.as_bytes())
                    .to_lowercase();

                local_attempts += 1;

                // Check if it matches
                if base32_id.starts_with(&prefix) {
                    found.store(true, Ordering::Relaxed);
                    total_attempts.fetch_add(local_attempts, Ordering::Relaxed);
                    let _ = tx.send(VanityResult {
                        secret_key,
                        endpoint_id,
                        base32_id,
                        attempts: total_attempts.load(Ordering::Relaxed),
                    });
                    break;
                }

                // Update progress periodically
                if local_attempts.is_multiple_of(10_000) {
                    total_attempts.fetch_add(10_000, Ordering::Relaxed);
                    local_attempts = 0;
                }

                // Check max attempts
                if let Some(max) = max_attempts
                    && total_attempts.load(Ordering::Relaxed) >= max
                {
                    found.store(true, Ordering::Relaxed);
                    break;
                }
            }
        });

        handles.push(handle);
    }

    // Drop the original sender so rx.recv() will return Err when all threads are done
    drop(tx);

    // Progress reporter (if not quiet)
    let progress_handle = if !quiet {
        let total_attempts = Arc::clone(&total_attempts);
        let found = Arc::clone(&found);
        Some(std::thread::spawn(move || {
            let mut last_count = 0;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if found.load(Ordering::Relaxed) {
                    break;
                }
                let current = total_attempts.load(Ordering::Relaxed);
                let rate = current.saturating_sub(last_count);
                last_count = current;
                print!(
                    "\rSearching... ({} attempts, {} keys/sec)  ",
                    format_number(current),
                    format_number(rate)
                );
                io::stdout().flush().ok();
            }
        }))
    } else {
        None
    };

    // Wait for result
    let result = match rx.recv() {
        Ok(result) => {
            if !quiet {
                println!("\r\n✓ Found matching key!                                   ");
                println!();
            }
            result
        }
        Err(_) => {
            // All threads finished without finding a match
            if !quiet {
                if let Some(max) = max_attempts {
                    println!("\r\n✗ No match found within {} attempts", max);
                } else {
                    println!("\r\n✗ No match found");
                }
            }
            return Err(anyhow!("No vanity address found"));
        }
    };

    // Wait for all threads to finish
    for handle in handles {
        handle.join().ok();
    }

    if let Some(h) = progress_handle {
        h.join().ok();
    }

    let elapsed = start_time.elapsed();
    let rate = result.attempts as f64 / elapsed.as_secs_f64();

    // Display result
    if !quiet {
        println!("Node ID:");
        println!("  Base32:  {}", result.base32_id);
        println!("  Hex:     {}", hex::encode(result.endpoint_id.as_bytes()));
        println!("  Domain:  {}.iron", result.base32_id);
        let ipv6 = iron::mapping::Registry::derive_ip(result.endpoint_id);
        println!("  IPv6:    {}", ipv6);
        println!();
        println!("Attempts:  {}", format_number(result.attempts));
        println!("Time:      {:.1} seconds", elapsed.as_secs_f64());
        println!("Rate:      {} keys/second", format_number(rate as u64));
        println!();

        // Always display the secret key so it's not lost
        println!("Secret Key (hex):");
        println!("  {}", hex::encode(result.secret_key.to_bytes()));
        println!();
        println!("⚠️  Save this secret key! It cannot be recovered if lost.");
        println!();
    } else {
        // Quiet mode: just output the base32 ID
        println!("{}", result.base32_id);
    }

    // Handle saving
    let should_save = if save || output.is_some() {
        true
    } else if !quiet {
        // Prompt to save interactively
        print!("Save this key now? (Y/n) ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let response = input.trim();
        response.is_empty() || response.eq_ignore_ascii_case("y")
    } else {
        false
    };

    if should_save {
        if let Some(output_path) = output {
            save_key_to_file(&result.secret_key, &output_path)?;
            if !quiet {
                println!("✓ Key saved to: {}", output_path);
            }
        } else {
            let key_path = keys::key_path();

            // Check if key already exists
            if key_path.exists() && !quiet {
                print!("\nWarning: This will overwrite your existing key. Continue? (y/N) ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled. Key not saved.");
                    println!();
                    println!("Secret key (save manually if needed):");
                    println!("  {}", hex::encode(result.secret_key.to_bytes()));
                    return Ok(());
                }
            }

            // Try to save the key
            let key_path_str = key_path
                .to_str()
                .ok_or_else(|| anyhow!("Key path contains invalid UTF-8"))?;
            match save_key_to_file(&result.secret_key, key_path_str) {
                Ok(_) => {
                    if !quiet {
                        println!("✓ Key saved to: {}", key_path.display());
                    }
                }
                Err(e) if e.to_string().contains("Permission denied") => {
                    println!();
                    println!("⚠️  Permission denied. The key directory may be owned by root.");
                    println!();
                    println!("Solutions:");
                    println!("1. Run iron daemon once (as root) to fix permissions:");
                    println!("   sudo iron serve");
                    println!("   (then Ctrl-C and try vanity again)");
                    println!();
                    println!("2. Save to a custom location:");
                    println!("   iron vanity {} --output ~/my-key.secret", prefix);
                    println!();
                    println!("Secret key (save manually if needed):");
                    println!("  {}", hex::encode(result.secret_key.to_bytes()));
                }
                Err(e) => return Err(e),
            }
        }
    }

    Ok(())
}

struct VanityResult {
    secret_key: SecretKey,
    endpoint_id: iroh::EndpointId,
    base32_id: String,
    attempts: u64,
}

fn validate_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty() {
        return Err(anyhow!("Prefix cannot be empty"));
    }

    if prefix.len() > 8 {
        return Err(anyhow!(
            "Prefix too long (max 8 characters for reasonable difficulty)"
        ));
    }

    // Check for invalid base32 characters
    let valid_chars = "abcdefghijklmnopqrstuvwxyz234567";
    for c in prefix.chars() {
        if !valid_chars.contains(c.to_ascii_lowercase()) {
            return Err(anyhow!(
                "Invalid character '{}' in prefix. Base32 alphabet: a-z, 2-7",
                c
            ));
        }
    }

    // Warn about very long prefixes
    if prefix.len() >= 6 {
        let difficulty = 32_u64.pow(prefix.len() as u32);
        println!(
            "\n⚠️  WARNING: Prefix '{}' requires ~{} attempts",
            prefix,
            format_number(difficulty)
        );
        println!("   This may take a very long time!");
        print!("   Continue? (y/N) ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            return Err(anyhow!("Cancelled"));
        }
    }

    Ok(())
}

fn print_difficulty_estimate(prefix: &str) {
    let difficulty = 32_u64.pow(prefix.len() as u32);
    println!(
        "Difficulty: ~32^{} = {} attempts (estimated)",
        prefix.len(),
        format_number(difficulty)
    );
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000_000_000 {
        format!("{:.1}T", n as f64 / 1_000_000_000_000.0)
    } else if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn save_key_to_file(secret_key: &SecretKey, path: &str) -> Result<()> {
    let path_buf = std::path::PathBuf::from(path);

    // Create parent directory if it doesn't exist
    if let Some(parent) = path_buf.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write key file
    fs::write(&path_buf, secret_key.to_bytes())?;

    // Set permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path_buf, permissions)?;
    }

    Ok(())
}
