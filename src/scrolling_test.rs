use super::*;

#[test]
fn scroll_offset_clamps_to_bounds() {
    assert_eq!(scroll_offset(0, -1, 10), 0);
    assert_eq!(scroll_offset(3, -2, 10), 1);
    assert_eq!(scroll_offset(3, 2, 10), 5);
    assert_eq!(scroll_offset(9, 5, 10), 10);
}
