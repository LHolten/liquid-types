use super::Constraint;
use super::ExtendedConstraint;
use super::InnerTerm;
use super::Prop;
use super::Term;
use std::cmp::max;
use std::iter::zip;
use std::ops::BitAnd;
use std::rc::Rc;

pub(super) fn and(iter: impl IntoIterator<Item = ExtendedConstraint>) -> ExtendedConstraint {
    iter.into_iter()
        .fold(ExtendedConstraint::default(), BitAnd::bitand)
}

impl BitAnd for ExtendedConstraint {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        let Self { w: w1, r: mut r1 } = self;
        let Self { w: w2, r: r2 } = rhs;
        let new_len = max(r1.len(), r2.len());
        r1.resize_with(new_len, || None);
        for (r1, r2) in zip(&mut r1, r2) {
            *r1 = r1.take().or(r2)
        }
        Self {
            w: Rc::new(Constraint::And(w1, w2)),
            r: r1,
        }
    }
}

impl ExtendedConstraint {
    pub fn and_prop(mut self, prop: &Rc<Prop>) -> Self {
        let cons = Rc::new(Constraint::Prop(prop.clone()));
        self.w = Rc::new(Constraint::And(self.w, cons));
        self
    }

    // uses the found solution for the topmost variable
    pub fn push_down(mut self, evar: &Rc<Term>) -> Self {
        // create a new scope to make sure the TermRef is dropped
        let idx = {
            let InnerTerm::EVar(idx, _) = *evar.borrow() else {
                panic!()
            };
            idx
        };

        assert_eq!(self.r.len(), idx + 1);
        let Some(t) = self.r.pop().unwrap() else {
            panic!()
        };
        evar.value.set(Some(t.borrow().clone()));
        self
    }

    pub fn inst(mut self, t: &Rc<Term>, t_: &Rc<Term>) -> Self {
        let prop = Rc::new(Prop::Eq(t.clone(), t_.clone()));
        self = self.and_prop(&prop);
        if let InnerTerm::EVar(x, _) = *t_.borrow() {
            self.r.resize_with(max(self.r.len(), x + 1), || None);
            self.r[x] = self.r[x].take().or_else(|| Some(t.clone()));
        }
        self
    }
}
