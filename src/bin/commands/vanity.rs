use anyhow::{Result, anyhow};
use iroh::SecretKey;
use iron::keys;
use std::fs;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn run(
    prefix: String,
    threads: Option<usize>,
    max_attempts: Option<u64>,
    save: bool,
    output: Option<String>,
    quiet: bool,
) -> Result<()> {
    validate_prefix(&prefix)?;

    let prefix_lower = prefix.to_lowercase();
    let num_threads = resolve_thread_count(threads);

    if !quiet {
        print_search_banner(&prefix, num_threads, &prefix_lower);
    }

    let start_time = Instant::now();
    let result = run_search(&prefix_lower, num_threads, max_attempts, quiet)?;
    let elapsed = start_time.elapsed();

    display_result(&result, elapsed, quiet)?;
    handle_save(&result, save, output, quiet, &prefix)
}

fn resolve_thread_count(threads: Option<usize>) -> usize {
    threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    })
}

fn run_search(
    prefix: &str,
    num_threads: usize,
    max_attempts: Option<u64>,
    quiet: bool,
) -> Result<VanityResult> {
    let found = Arc::new(AtomicBool::new(false));
    let total_attempts = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::channel();

    let handles: Vec<_> = (0..num_threads)
        .map(|_| {
            let prefix = prefix.to_string();
            let found = Arc::clone(&found);
            let total_attempts = Arc::clone(&total_attempts);
            let tx = tx.clone();
            std::thread::spawn(move || {
                worker_loop(&prefix, max_attempts, &found, &total_attempts, tx)
            })
        })
        .collect();

    let progress_handle = (!quiet).then(|| {
        let total_attempts = Arc::clone(&total_attempts);
        let found = Arc::clone(&found);
        std::thread::spawn(move || progress_loop(&found, &total_attempts))
    });

    drop(tx);

    let result = match rx.recv() {
        Ok(result) => result,
        Err(_) => {
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

    for handle in handles {
        handle.join().ok();
    }
    if let Some(h) = progress_handle {
        h.join().ok();
    }

    Ok(result)
}

fn worker_loop(
    prefix: &str,
    max_attempts: Option<u64>,
    found: &AtomicBool,
    total_attempts: &AtomicU64,
    tx: mpsc::Sender<VanityResult>,
) {
    let mut local_attempts = 0u64;

    loop {
        if found.load(Ordering::Relaxed) {
            break;
        }

        let secret_key = SecretKey::generate();
        let endpoint_id = secret_key.public();
        let base32_id = data_encoding::BASE32_NOPAD
            .encode(endpoint_id.as_bytes())
            .to_lowercase();

        local_attempts += 1;

        if base32_id.starts_with(prefix) {
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

        if local_attempts.is_multiple_of(10_000) {
            total_attempts.fetch_add(10_000, Ordering::Relaxed);
            local_attempts = 0;
        }

        if let Some(max) = max_attempts
            && total_attempts.load(Ordering::Relaxed) >= max
        {
            found.store(true, Ordering::Relaxed);
            break;
        }
    }
}

fn progress_loop(found: &AtomicBool, total_attempts: &AtomicU64) {
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
}

fn print_search_banner(prefix: &str, num_threads: usize, prefix_lower: &str) {
    println!(
        "\nSearching for vanity address with prefix \"{}\"...",
        prefix
    );
    println!("Threads: {}", num_threads);
    print_difficulty_estimate(prefix_lower);
    println!();
}

fn display_result(result: &VanityResult, elapsed: Duration, quiet: bool) -> Result<()> {
    if !quiet {
        println!("\r\n✓ Found matching key!                                   ");
        println!();
        let rate = result.attempts as f64 / elapsed.as_secs_f64();

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
        println!("Secret Key (hex):");
        println!("  {}", hex::encode(result.secret_key.to_bytes()));
        println!();
        println!("⚠️  Save this secret key! It cannot be recovered if lost.");
        println!();
    } else {
        println!("{}", result.base32_id);
    }
    Ok(())
}

fn handle_save(
    result: &VanityResult,
    save: bool,
    output: Option<String>,
    quiet: bool,
    prefix: &str,
) -> Result<()> {
    let should_save = if save || output.is_some() {
        true
    } else if !quiet {
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

    let valid_chars = "abcdefghijklmnopqrstuvwxyz234567";
    for c in prefix.chars() {
        if !valid_chars.contains(c.to_ascii_lowercase()) {
            return Err(anyhow!(
                "Invalid character '{}' in prefix. Base32 alphabet: a-z, 2-7",
                c
            ));
        }
    }

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

    if let Some(parent) = path_buf.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&path_buf, secret_key.to_bytes())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path_buf, permissions)?;
    }

    Ok(())
}
