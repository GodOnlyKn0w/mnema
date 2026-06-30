pub(in crate::tests) fn long_summary() -> String {
    format!(
        "{}{}",
        "a".repeat(50),
        "测试摘要内容验证把手完整性规则不截断标识符".repeat(3),
    )
}
