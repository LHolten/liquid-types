use std::{
    collections::HashMap,
    iter::zip,
    rc::{Rc, UniqueRc, Weak},
};

use crate::{
    parse::expr::{IfZero, Let, Stmt},
    refinement::{
        self,
        heap::{BoolFuncTerm, Heap},
    },
};

use super::{
    expr::{BinOpValue, Block, Def, FuncDef, Module, Value},
    types::{Constraint, NamedConstraint, NegTyp, PosTyp, Prop, PropOp},
};

#[derive(Clone)]
pub struct Desugar {
    named: WeakNameList,
    terms: HashMap<String, refinement::Term>,
    vars: HashMap<String, refinement::Local<refinement::Var>>,
    labels: HashMap<String, refinement::Var>,
}

impl Desugar {
    pub fn new(named: WeakNameList) -> Self {
        Self {
            named,
            terms: HashMap::new(),
            vars: HashMap::new(),
            labels: HashMap::new(),
        }
    }
}

type LazyName = Box<dyn Fn(WeakNameList) -> refinement::Name>;

#[derive(Default)]
struct LazyNameList(HashMap<String, (UniqueRc<refinement::Name>, LazyName)>);

#[derive(Clone)]
struct WeakNameList(HashMap<String, Weak<refinement::Name>>);

impl LazyNameList {
    pub fn weak(&self) -> WeakNameList {
        let iter = self.0.iter();
        let iter = iter.map(|(k, v)| (k.clone(), UniqueRc::downgrade(&v.0)));
        WeakNameList(iter.collect())
    }

    pub fn fix(mut self) -> HashMap<String, Rc<refinement::Name>> {
        let list = self.weak();

        self.0.values_mut().for_each(|v| *v.0 = (v.1)(list.clone()));
        self.0
            .into_iter()
            .map(|(k, v)| (k, UniqueRc::into_rc(v.0)))
            .collect()
    }
}

impl Value {
    pub fn convert<T: Clone>(&self, lookup: &HashMap<String, T>) -> refinement::Free<T> {
        match self {
            Value::Var(name) => refinement::Free::Var(lookup.get(name).unwrap().clone()),
            Value::Int32(val) => refinement::Free::Just(*val as i64, 32),
            Value::BinOp(binop) => binop.convert(lookup),
            Value::Prop(prop) => prop.convert(lookup),
        }
    }
}

impl BinOpValue {
    pub fn convert<T: Clone>(&self, lookup: &HashMap<String, T>) -> refinement::Free<T> {
        let op = match self.op {
            super::expr::BinOp::Plus => refinement::BinOp::Add,
            super::expr::BinOp::Minus => refinement::BinOp::Sub,
        };
        refinement::Free::BinOp {
            l: Rc::new(self.l.convert(lookup)),
            r: Rc::new(self.r.convert(lookup)),
            op,
        }
    }
}

impl refinement::BinOp {
    pub fn free<T>(self, l: refinement::Free<T>, r: refinement::Free<T>) -> refinement::Free<T> {
        refinement::Free::BinOp {
            l: Rc::new(l),
            r: Rc::new(r),
            op: self,
        }
    }
}

impl<T> refinement::Free<T> {
    pub fn bool_not(self) -> Self {
        refinement::BinOp::Sub.free(Self::Just(1, 32), self)
    }
}

impl Prop {
    pub fn convert<T: Clone>(&self, lookup: &HashMap<String, T>) -> refinement::Free<T> {
        let l = self.l.convert(lookup);
        let r = self.r.convert(lookup);
        use refinement::BinOp as Op;
        match self.op {
            PropOp::Less => Op::Less.free(l, r),
            PropOp::LessEq => Op::Less.free(r, l).bool_not(),
            PropOp::Eq => Op::Eq.free(l, r),
            PropOp::NotEq => Op::Eq.free(l, r).bool_not(),
            PropOp::And => Op::And.free(l, r),
        }
    }
}

impl Desugar {
    pub fn convert_pos(&self, pos: Rc<PosTyp>) -> refinement::Fun<refinement::PosTyp> {
        let this = self.clone();
        refinement::Fun {
            tau: pos.names.iter().map(|_| 32).collect(),
            fun: Rc::new(move |heap, terms| {
                let mut this = this.clone();
                this.terms.extend(zip(pos.names.clone(), terms.to_owned()));

                this.convert_constraint(&pos.parts, heap);

                refinement::PosTyp
            }),
        }
    }

    pub fn convert_neg(&self, neg: NegTyp) -> refinement::Fun<refinement::NegTyp> {
        let NegTyp { args, ret } = neg;

        let this = self.clone();
        refinement::Fun {
            tau: args.names.iter().map(|_| 32).collect(),
            fun: Rc::new(move |heap, terms| {
                let mut this = this.clone();
                this.terms.extend(zip(args.names.clone(), terms.to_owned()));

                this.convert_constraint(&args.parts, heap);

                refinement::NegTyp {
                    arg: refinement::PosTyp,
                    ret: this.convert_pos(ret.clone()),
                }
            }),
        }
    }

    pub fn convert_prop(&self, prop: &Prop) -> refinement::Term {
        prop.convert(&self.terms).make_term()
    }

    pub fn convert_binop(&self, binop: &BinOpValue) -> refinement::Term {
        binop.convert(&self.terms).make_term()
    }

    pub fn convert_val(&self, val: &Value) -> refinement::Term {
        val.convert(&self.terms).make_term()
    }

    pub fn convert_vals(&self, vals: &[Value]) -> Vec<refinement::Term> {
        vals.iter().map(|x| self.convert_val(x)).collect()
    }

    pub fn convert_constraint(&mut self, parts: &[Constraint], heap: &mut dyn Heap) {
        for part in parts {
            match part {
                Constraint::Forall(forall) => {
                    let named = self.named.0.get(&forall.named).unwrap();
                    let cond = forall.cond.clone();
                    let names = forall.names.clone();
                    let this = self.clone();
                    heap.forall(refinement::Forall {
                        named: named.clone(),
                        mask: BoolFuncTerm::new(move |terms| {
                            let mut this = this.clone();
                            this.terms.extend(zip(names.clone(), terms.to_owned()));
                            this.convert_prop(&cond).to_bool()
                        }),
                    });
                }
                Constraint::Assert(cond) => heap.assert(self.convert_prop(cond)),
                Constraint::Builtin(new_name, call) => {
                    let name = call.func.as_ref().unwrap();
                    if name.starts_with('@') {
                        assert_eq!(name, "@byte");
                        let [ptr] = &*call.args else { panic!() };
                        let heap_val = heap.owned(&self.convert_val(ptr), 1, 32);
                        self.terms.insert(new_name.to_owned(), heap_val);
                    } else {
                        let named = self.named.0.get(name).unwrap().upgrade().unwrap();
                        (named.typ.fun)(heap, &self.convert_vals(&call.args));
                    }
                }
            }
        }
    }

    pub fn convert_value(&self, value: &[Value]) -> refinement::Value<refinement::Var> {
        refinement::Value {
            inj: value.iter().map(|val| val.convert(&self.vars)).collect(),
        }
    }

    pub fn convert_lambda(
        &self,
        index: usize,
        block: &Rc<Block>,
        label: Option<String>,
        names: Vec<String>,
    ) -> refinement::Lambda<refinement::Var> {
        let this = self.clone();
        let block = block.clone();
        refinement::Lambda(Rc::new(move |arg| -> refinement::Expr<refinement::Var> {
            let mut this = this.clone();
            let locals = (0..).map(|i| refinement::Local(arg.clone(), i));
            this.vars.extend(zip(names.clone(), locals));
            if let Some(label) = label.as_ref() {
                this.labels.insert(label.clone(), arg.clone());
            }

            let Some(step) = block.steps.get(index) else {
                let value = this.convert_value(&block.end.args);
                return match block.end.func.as_ref() {
                    Some(func) => {
                        let label = this.labels.get(func).unwrap();
                        refinement::Expr::Loop(label.clone(), Rc::new(value))
                    }
                    None => refinement::Expr::Return(Rc::new(value)),
                };
            };

            match step {
                Stmt::Let(Let { label, names, bind }) => {
                    let rest = this.convert_lambda(index + 1, &block, label.clone(), names.clone());

                    let func_name = bind.func.as_ref().unwrap();
                    let func = if func_name.starts_with('@') {
                        let builtin = match func_name.as_str() {
                            "@read_u8" => refinement::builtin::Builtin::Read,
                            "@write_u8" => refinement::builtin::Builtin::Write,
                            _ => panic!(),
                        };
                        refinement::Thunk::Builtin(builtin)
                    } else {
                        let local = this.vars.get(func_name).unwrap();
                        refinement::Thunk::Local(local.clone())
                    };
                    let arg = this.convert_value(&bind.args);
                    let bound = refinement::BoundExpr::App(func, Rc::new(arg));

                    refinement::Expr::Let(bound, rest)
                }
                Stmt::FuncDef(FuncDef {
                    name,
                    typ,
                    block: def,
                }) => {
                    let rest = this.convert_lambda(index + 1, &block, Some(name.clone()), vec![]);
                    let cont = this.convert_lambda(0, def, Some(name.clone()), vec![]);
                    let bound = refinement::BoundExpr::Cont(cont, this.convert_neg(typ.clone()));
                    refinement::Expr::Let(bound, rest)
                }
                Stmt::IfZero(IfZero { val, block: def }) => {
                    let rest = this.convert_lambda(index + 1, &block, None, vec![]);
                    let cont = this.convert_lambda(0, def, None, vec![]);

                    let local = val.convert(&this.vars);
                    refinement::Expr::Match(local, vec![cont, rest])
                }
                Stmt::Unpack(bind) => {
                    let func = this.named.0.get(bind.func.as_ref().unwrap()).unwrap();
                    let rest = this.convert_lambda(index + 1, &block, None, vec![]);

                    let arg = this.convert_value(&bind.args);
                    let builtin = refinement::builtin::Builtin::Pack(func.clone(), true);
                    let func = refinement::Thunk::Builtin(builtin);
                    let bound = refinement::BoundExpr::App(func, Rc::new(arg));
                    refinement::Expr::Let(bound, rest)
                }
                Stmt::Pack(bind) => {
                    let func = this.named.0.get(bind.func.as_ref().unwrap()).unwrap();
                    let rest = this.convert_lambda(index + 1, &block, None, vec![]);

                    let arg = this.convert_value(&bind.args);
                    let builtin = refinement::builtin::Builtin::Pack(func.clone(), false);
                    let func = refinement::Thunk::Builtin(builtin);
                    let bound = refinement::BoundExpr::App(func, Rc::new(arg));
                    refinement::Expr::Let(bound, rest)
                }
            }
        }))
    }

    pub fn check(m: Module) {
        let mut list = LazyNameList::default();

        for def in &m.0 {
            match def {
                Def::Func(_func) => {}
                Def::Typ(named) => {
                    let NamedConstraint { name, typ } = named.clone();
                    let fun = refinement::Fun {
                        tau: vec![],
                        fun: Rc::new(|_, _| refinement::PosTyp),
                    };

                    let delayed = Box::new(move |named| {
                        let this = Desugar::new(named);
                        let pos = this.convert_pos(typ.clone());
                        refinement::Name::new(pos)
                    });
                    list.0
                        .insert(name, (UniqueRc::new(refinement::Name::new(fun)), delayed));
                }
            }
        }

        let this = Desugar::new(list.weak());
        let list = list.fix();

        for def in m.0 {
            match def {
                Def::Func(func) => {
                    let neg = this.convert_neg(func.typ.clone());
                    let lambda = this.convert_lambda(
                        0,
                        &func.block,
                        Some(func.name),
                        func.typ.args.names.clone(),
                    );

                    let ctx = refinement::SubContext::default();
                    ctx.check_expr(&lambda, &neg);
                }
                Def::Typ(_named) => {}
            }
        }

        drop(list)
    }
}
