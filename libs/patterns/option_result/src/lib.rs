#![allow(dead_code)]
// Using Option for something that might not exist
fn get_username_by_id(id: u64) -> Option<String> {
    match id {
        1 => Some("Alice".to_string()),
        2 => Some("Bob".to_string()),
        _ => None, // no user found for that id
    }
}

// Using Result for an operation that might, like, fail
fn parse_port(port: &str) -> Result<u32, String> {
    port.parse::<u32>()
        .map_err(|e| format!("Invalid port format: {e}"))
        .and_then(|port| {
            if port > 1023 && port <= 65535 {
                Ok(port)
            } else {
                Err(format!("Port {port} is outside valid range (1024-65535)"))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        match get_username_by_id(1) {
            None => println!("User not found!"),
            Some(name) => println!("Found user: {name}"),
        }
        match get_username_by_id(3) {
            Some(name) => println!("Found user: {name}"),
            None => println!("User not found!"),
        }
        println!("---");
        // Result example next!
        match parse_port("8080") {
            Ok(port) => println!("Successfully parsed port: {port}"),
            Err(e) => eprintln!("Error parsing port: {e}"),
        }
        match parse_port("invalid") {
            Ok(port) => println!("Successfully parsed port: {port}"),
            Err(e) => eprintln!("Error parsing port: {e}"),
        }
        match parse_port("80") {
            Ok(port) => println!("Successfully parsed port: {port}"),
            Err(e) => eprintln!("Error parsing port: {e}"),
        }
    }
}
