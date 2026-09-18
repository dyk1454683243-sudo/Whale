#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    Absolute64,
    Absolute32,
    Relative32,
    Relative8,
    GOTPCREL,
    PLT32,
}

#[derive(Debug, Clone)]
pub struct ObjectRelocation {
    pub section_index: usize,
    pub offset: usize,
    pub symbol: String,
    pub addend: i64,
    pub kind: RelocKind,
}
