//! The producer must retain every declaration policy, not infer it from use.
pub const OWNER: &str = "fixture.visibility.owner";
pub const SOURCE: &str = r#"
module fixture.visibility.owner;
public type Vault is {
    implicit_private: Int,
    private explicit_private: Int,
    public visible: Int,
    public(cog) cog_only: Int,
    public(super) parent_only: Int,
    public(in fixture.visibility) scoped: Int,
    public(in super) relative_scoped: Int,
    internal internal_only: Int,
    protected protected_only: Int,
};
"#;
