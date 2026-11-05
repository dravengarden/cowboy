use super::question_summary_title;

#[test]
fn empty_page_titles_use_page_view_terminology() {
    assert_eq!(question_summary_title("7", 7), "Page 7");
    assert_eq!(question_summary_title("###", 12), "Page 12");
}
