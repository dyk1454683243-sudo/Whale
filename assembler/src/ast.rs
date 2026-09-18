#[derive(Debug, Clone)]
pub struct AST {
    pub items: Vec<ASTNode>,
}

#[derive(Debug, Clone)]
pub enum ASTNode {
    Instruction(Instruction),
    Directive(Directive),
    Label(String),
    Section(String),
    Global(String),
    Extern(String),
    Const { name: String, expr: ExprValue },
}

#[derive(Debug, Clone)]
pub struct Instruction {
    pub mnemonic: String,
    pub operands: Vec<Operand>,
}

#[derive(Debug, Clone)]
pub struct Directive {
    pub name: String,
    pub values: Vec<DirectiveValue>,
}

#[derive(Debug, Clone)]
pub enum DirectiveValue {
    Expr(ExprValue),
    StringLiteral(String),
}

#[derive(Debug, Clone)]
pub enum ExprValue {
    Number(i64),
    Symbol { name: String, addend: i64 },
}

#[derive(Debug, Clone)]
pub enum Operand {
    Register(String),
    Immediate(i64),
    Label(String),
    SymbolExpr { name: String, addend: i64 },
    Memory(MemoryOperand)
}

#[derive(Debug, Clone)]
pub struct MemoryOperand {
    pub base: Option<String>,
    pub index: Option<String>,
    pub scale: u8,
    pub disp: i64,
    pub symbol: Option<String>,
}
