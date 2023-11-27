use std::{
    iter::zip,
    mem::forget,
    rc::{Rc, Weak},
};

use crate::refinement::Free;

use super::{BoundExpr, Expr, Fun, Lambda, NegTyp, PosTyp, SubContext, Term, Thunk, Val, Value};

pub fn zip_eq<A: IntoIterator, B: IntoIterator>(
    a: A,
    b: B,
) -> impl Iterator<Item = (A::Item, B::Item)>
where
    A::IntoIter: ExactSizeIterator,
    B::IntoIter: ExactSizeIterator,
{
    let (a_iter, b_iter) = (a.into_iter(), b.into_iter());
    assert_eq!(a_iter.len(), b_iter.len());
    zip(a_iter, b_iter)
}

impl Fun<PosTyp> {
    pub fn arrow(self, ret: Fun<PosTyp>) -> Fun<NegTyp> {
        Fun {
            tau: self.tau,
            fun: Rc::new(move |args, heap| NegTyp {
                arg: (self.fun)(args, heap),
                ret: ret.clone(),
            }),
        }
    }
}

impl Val for Term {
    type Func = Fun<NegTyp>;
    fn make(_lamb: &Weak<Lambda<Self>>, typ: &Fun<NegTyp>) -> Self::Func {
        typ.clone()
    }
}

impl SubContext {
    fn infer_func(&self, func: &Thunk<Term>) -> Fun<NegTyp> {
        match func {
            Thunk::Local(local) => local.clone(),
            Thunk::Builtin(builtin) => builtin.infer(),
        }
    }

    fn calc_args(&self, val: &Value<Term>) -> Vec<Term> {
        let mut res = vec![];
        for inj in &val.inj {
            res.push(self.check_free(inj))
        }
        res
    }

    pub fn check_free(&self, free: &Free<Term>) -> Term {
        match free {
            Free::Just(idx, size) => Term::nat(*idx, *size),
            Free::Var(local) => local.clone(),
            Free::BinOp { l, r, op } => {
                let (l, r) = (self.check_free(l), self.check_free(r));
                self.check_binop(op, &l, &r);
                op.apply(&l, &r)
            }
        }
    }

    // This resolves value determined indices in `p`
    pub fn check_value(&mut self, v: &Value<Term>, p: &Fun<PosTyp>) {
        let p_args = self.calc_args(v);
        let PosTyp = self.with_terms(p, &p_args);
    }

    pub fn spine(&mut self, n: &Fun<NegTyp>, s: &Value<Term>) -> Fun<PosTyp> {
        let n_args = self.calc_args(s);
        let typ = self.with_terms(n, &n_args);
        typ.ret
    }

    pub fn check_expr(mut self, l: &Lambda<Term>, n: &Fun<NegTyp>) {
        let neg = self.extract(n);
        let e = l.inst(&neg.terms);
        self.check_expr_pos(&e, &neg.inner.ret);
    }

    pub fn check_expr_pos(mut self, e: &Expr<Term>, p: &Fun<PosTyp>) {
        match e {
            Expr::Return(v) => {
                self.check_value(v, p);
                self.check_empty();
            }
            Expr::Let(g, l) => {
                match g {
                    BoundExpr::App(func, s) => {
                        let n = self.infer_func(func);
                        let bound_p = self.spine(&n, s);
                        self.check_expr(l, &bound_p.arrow(p.clone()))
                    }
                    BoundExpr::Cont(c, n) => {
                        self.clone().check_expr(c, n);
                        let e = l.inst(&[]);
                        self.check_expr_pos(&e, p);
                    }
                };
            }
            Expr::Match(free, pats) => {
                let term = self.check_free(free);
                let size = term.get_size();
                let (last, pats) = pats.split_last().unwrap();

                for (i, l) in pats.iter().enumerate() {
                    // we want to preserve resources between branches
                    let this = self.clone();
                    let phi = term.eq(&Term::nat(i as i64, size));
                    let match_p = this.unroll_prod_univ(phi);
                    this.check_expr(l, &match_p.arrow(p.clone()));
                }

                let phi = Term::nat(pats.len() as i64, size).ule(&term);
                let match_p = self.unroll_prod_univ(phi);
                self.check_expr(last, &match_p.arrow(p.clone()));
            }
            Expr::Loop(n, s) => {
                let res = self.spine(n, s);
                self.sub_pos_typ(&res, p);
            }
        }
    }

    pub fn without_alloc(&self) -> Self {
        Self {
            assume: self.assume.clone(),
            forall: vec![],
        }
    }

    pub fn check_empty(mut self) {
        // leaking resources is not allowed
        // TODO: make sure this doesn't leak
        assert!(self.forall.is_empty(), "can not leak resources");
        self.assume.clear();
        forget(self);
    }
}
