//! List cameras and capture cards without opening a capture session.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = virtual_media::discover_cameras()?;
    println!("{} video input(s)", devices.len());
    for device in devices {
        println!("{}\t{}", device.label, device.id);
    }
    Ok(())
}
