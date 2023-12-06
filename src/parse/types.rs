//! this is for a separate type alias files

use std::rc::Rc;

use super::expr::{Bind, Spanned, Value};

#[derive(Clone)]
pub struct NamedConstraint {
    pub name: String,
    pub typ: Rc<Spanned<PosTyp>>,
}

pub struct PosTyp {
    pub names: Vec<String>,
    pub parts: Vec<Spanned<Constraint>>,
}

#[derive(Clone)]
pub struct NegTyp {
    pub args: Rc<Spanned<PosTyp>>,
    pub ret: Rc<Spanned<PosTyp>>,
}

pub enum Constraint {
    Forall(Forall),
    Assert(Prop),
    // Func(Term, NegTyp),
    Builtin(Option<String>, Bind),
}

pub struct Forall {
    pub named: String,
    pub names: Vec<String>,
    pub cond: Rc<Prop>,
}

pub struct Switch {
    pub cond: Prop,
    pub named: String,
    pub args: Vec<Value>,
}

pub enum PropOp {
    Less,
    LessEq,
    Eq,
    NotEq,
    And,
    MulSafe,
}

pub struct Prop {
    pub l: Value,
    pub r: Value,
    pub op: PropOp,
}
