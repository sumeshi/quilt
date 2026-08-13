/// Parse a batch size string like `512MB`, `2GB` into bytes.
pub fn parse_batch_size(size_str: &str) -> Result<usize, String> {
    let size_str = size_str.trim().to_uppercase();
    if size_str.is_empty() {
        return Ok(1_073_741_824);
    }
    let (number_str, unit) = if size_str.ends_with("GB") {
        (&size_str[..size_str.len() - 2], "GB")
    } else if size_str.ends_with("MB") {
        (&size_str[..size_str.len() - 2], "MB")
    } else if size_str.ends_with("KB") {
        (&size_str[..size_str.len() - 2], "KB")
    } else if size_str.ends_with('B') {
        (&size_str[..size_str.len() - 1], "B")
    } else {
        (size_str.as_str(), "B")
    };
    let number: f64 = number_str
        .parse()
        .map_err(|_| format!("Invalid number in batch size: {size_str}"))?;
    if number < 0.0 {
        return Err("Batch size cannot be negative".into());
    }
    let bytes = match unit {
        "B" => number as usize,
        "KB" => (number * 1024.0) as usize,
        "MB" => (number * 1_048_576.0) as usize,
        "GB" => (number * 1_073_741_824.0) as usize,
        _ => return Err(format!("Unknown unit: {unit}")),
    };
    if bytes < 1_048_576 {
        return Err(format!(
            "Batch size too small (minimum: 1MB), got: {size_str}"
        ));
    }
    if bytes > 10_737_418_240 {
        return Err(format!(
            "Batch size too large (maximum: 10GB), got: {size_str}"
        ));
    }
    Ok(bytes)
}
