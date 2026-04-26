pub fn scroll_offset(current: usize, delta: isize, max: usize) -> usize {
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize).min(max)
    }
}

pub fn page_delta(page_height: usize, direction: isize) -> isize {
    direction.saturating_mul(page_height as isize)
}

#[cfg(test)]
#[path = "scrolling_test.rs"]
mod scrolling_test;
