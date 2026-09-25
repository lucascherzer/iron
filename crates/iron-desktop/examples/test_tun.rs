use tun::{AbstractDevice, Configuration, Layer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing TUN device creation...");

    let mut config = Configuration::default();
    config
        .layer(Layer::L3)
        .address((169, 254, 0, 1))
        .destination((169, 254, 0, 2))
        .mtu(1420)
        .up();

    #[cfg(target_os = "macos")]
    config.tun_name("utun");

    #[cfg(target_os = "linux")]
    config.tun_name("iron0");

    println!("Configuration: {:?}", config);

    match tun::create(&config) {
        Ok(device) => {
            println!("✓ TUN device created successfully!");
            println!("  Name: {}", device.tun_name()?);
            Ok(())
        }
        Err(e) => {
            eprintln!("✗ Failed to create TUN device: {:?}", e);
            eprintln!("  Error details: {}", e);
            Err(e.into())
        }
    }
}
