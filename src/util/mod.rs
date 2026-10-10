pub mod media;
pub mod player;
pub mod youtube;

pub fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

pub fn progress_bar(position: u64, duration: u64) -> String {
    let ratio = if duration == 0 {
        0.0
    } else {
        (position as f64 / duration as f64).clamp(0.0, 1.0)
    };
    let marker = (ratio * 30.0).round() as usize;
    format!(
        "{}🔘{}",
        "▬".repeat(marker),
        "▬".repeat(30usize.saturating_sub(marker))
    )
}

pub fn parse_time(value: &str) -> Option<u64> {
    let mut total = 0_u64;
    for part in value.split(':') {
        total = total.checked_mul(60)?.checked_add(part.parse().ok()?)?;
    }
    Some(total)
}

pub fn parse_iso_duration(value: &str) -> Option<u64> {
    let mut total = 0_u64;
    let mut number = String::new();
    let mut in_time = false;
    let mut saw_period = false;
    for ch in value.chars() {
        match ch {
            'P' if !saw_period && !in_time && number.is_empty() => saw_period = true,
            'T' if saw_period && !in_time && number.is_empty() => in_time = true,
            '0'..='9' => number.push(ch),
            'D' if saw_period && !in_time => {
                total = total.checked_add(number.parse::<u64>().ok()?.checked_mul(86_400)?)?;
                number.clear();
            }
            'H' if in_time => {
                total = total.checked_add(number.parse::<u64>().ok()?.checked_mul(3_600)?)?;
                number.clear();
            }
            'M' if in_time => {
                total = total.checked_add(number.parse::<u64>().ok()?.checked_mul(60)?)?;
                number.clear();
            }
            'S' if in_time => {
                total = total.checked_add(number.parse::<u64>().ok()?)?;
                number.clear();
            }
            'Y' | 'M' | 'W' => return None,
            _ => return None,
        }
    }
    (saw_period && number.is_empty()).then_some(total)
}
