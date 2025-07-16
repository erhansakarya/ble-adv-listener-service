use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use std::error::Error;
use tokio::time::{sleep, Duration};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug)]
struct ShellyBluMotionData {
    device_id: String,
    motion: Option<bool>,
    illuminance: Option<f32>,
    battery: Option<u8>,
    button_event: Option<u16>,
    timestamp: u64,
}

fn parse_bthome_data(data: &[u8]) -> (Option<bool>, Option<f32>, Option<u8>, Option<u16>) {
    let mut motion = None;
    let mut illuminance = None;
    let mut battery = None;
    let mut button_event = None;
    let mut i = 0;
    while i < data.len() {
        let id = data[i];
        i += 1;
        match id {
            0x00 => { // packet id, 1 byte
                i += 1;
            }
            0x01 => { // battery, 1 byte
                if i < data.len() {
                    battery = Some(data[i]);
                    i += 1;
                }
            }
            0x05 => { // illuminance, 3 bytes, uint24, scale 0.01
                if i + 2 < data.len() {
                    let lux_raw = (data[i] as u32) | ((data[i+1] as u32) << 8) | ((data[i+2] as u32) << 16);
                    illuminance = Some(lux_raw as f32 * 0.01);
                    i += 3;
                }
            }
            0x21 => { // motion, 1 byte
                if i < data.len() {
                    motion = Some(data[i] != 0);
                    i += 1;
                }
            }
            0x3A => { // button event, 2 bytes
                if i + 1 < data.len() {
                    button_event = Some((data[i] as u16) | ((data[i+1] as u16) << 8));
                    i += 2;
                }
            }
            _ => {
                // Unknown or unsupported, try to skip 1 byte
                i += 1;
            }
        }
    }
    (motion, illuminance, battery, button_event)
}

fn parse_shelly_blu_motion_data(manufacturer_data: &HashMap<u16, Vec<u8>>) -> Option<ShellyBluMotionData> {
    // Shelly BLU devices use manufacturer ID 2985 (0x0BA9)
    if let Some(data) = manufacturer_data.get(&2985) {
        if data.len() < 8 {
            return None;
        }
        // Device ID is usually the last 6 bytes (reverse order)
        let device_id = if data.len() >= 8 {
            format!(
                "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                data[data.len()-1], data[data.len()-2], data[data.len()-3],
                data[data.len()-4], data[data.len()-5], data[data.len()-6]
            )
        } else {
            "Unknown".to_string()
        };
        let (motion, illuminance, battery, button_event) = parse_bthome_data(data);
        Some(ShellyBluMotionData {
            device_id,
            motion,
            illuminance,
            battery,
            button_event,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    } else {
        None
    }
}

fn parse_bthome_service_data(data: &[u8]) {
    let mut i = 0;
    while i < data.len() {
        let id = data[i];
        i += 1;
        match id {
            0x00 => { // packet id
                if i < data.len() {
                    println!("  Packet ID: {}", data[i]);
                    i += 1;
                }
            }
            0x01 => { // battery
                if i < data.len() {
                    println!("  🔋 Battery: {}%", data[i]);
                    i += 1;
                }
            }
            0x05 => { // illuminance (3 bytes, uint24, scale 0.01)
                if i + 2 < data.len() {
                    let lux = (data[i] as u32) | ((data[i+1] as u32) << 8) | ((data[i+2] as u32) << 16);
                    println!("  💡 Illuminance: {:.2} lux", lux as f32 * 0.01);
                    i += 3;
                }
            }
            0x21 => { // motion
                if i < data.len() {
                    println!("  👁️  Motion: {}", if data[i] != 0 { "DETECTED" } else { "No Motion" });
                    i += 1;
                }
            }
            _ => {
                println!("  Unknown ID: 0x{:02X}", id);
                if i < data.len() { i += 1; }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let adapter = adapters.into_iter().nth(0).expect("No Bluetooth adapter found");

    println!("Starting continuous BLE scan for ALL devices...");
    println!("Press Ctrl+C to stop");

    adapter.start_scan(ScanFilter::default()).await?;

    loop {
        sleep(Duration::from_secs(5)).await;

        let peripherals = adapter.peripherals().await?;
        println!("\n=== Scan Cycle ===");
        println!("Found {} devices", peripherals.len());
        
        for peripheral in peripherals {
            if let Some(props) = peripheral.properties().await? {
                let address = peripheral.address();
                let rssi = props.rssi.map(|r| r.to_string()).unwrap_or_else(|| "N/A".to_string());
                
                println!("\nDevice: {} | RSSI: {}", address, rssi);
                
                // Print device name if available
                if let Some(name) = &props.local_name {
                    println!("  Name: {}", name);
                }
                
                // Print ALL manufacturer data
                for (id, data) in &props.manufacturer_data {
                    println!("  Manufacturer ID: 0x{:04X} | Data: {:?}", id, data);
                }
                
                // Print ALL service data
                let shelly_service_uuid = Uuid::parse_str("0000fcd2-0000-1000-8000-00805f9b34fb").unwrap();

                for (uuid, data) in &props.service_data {
                    println!("  Service Data UUID: {} | Data: {:?}", uuid, data);
                    if *uuid == shelly_service_uuid {
                        println!("  *** SHELLY BLU MOTION SERVICE DATA FOUND ***");
                        parse_bthome_service_data(data);
                    }
                }
                
                // Print ALL service UUIDs
                if !props.services.is_empty() {
                    println!("  Services: {:?}", props.services);
                }
                
                // Check if this might be our Shelly device
                if let Some(name) = &props.local_name {
                    if name.contains("SBM") || name.contains("Shelly") {
                        println!("  *** POTENTIAL SHELLY DEVICE FOUND ***");
                    }
                }
                
                // Check for Alterco Robotics manufacturer data
                if props.manufacturer_data.contains_key(&0x0BA9) {
                    println!("  *** ALTERCO ROBOTICS DEVICE FOUND ***");
                }

                let target_mac = "B0:C7:DE:7E:77:A0";
                if address.to_string() == target_mac {
                    println!("  >>> FOUND SHELLY BLU MOTION SENSOR <<<");
                    // Print all manufacturer and service data as before
                    for (id, data) in &props.manufacturer_data {
                        println!("  Manufacturer ID: 0x{:04X} | Data: {:?}", id, data);
                    }
                    for (uuid, data) in &props.service_data {
                        println!("  Service Data UUID: {} | Data: {:?}", uuid, data);
                    }
                }
            }
        }
    }
}
