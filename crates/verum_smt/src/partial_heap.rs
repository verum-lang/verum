//! Shared extensional partial-heap algebra for the word and mathematical-integer
//! frontends. Scalar interpretation belongs to the frontend; ownership, splitting,
//! extension and opaque spatial relations have one encoding here.

use verum_common::{List, Text};
use z3::{
    Sort,
    ast::{Array, Ast, Bool, Dynamic},
};

#[derive(Clone, Debug)]
pub struct PartialHeap {
    values: Array,
    owned: Array,
    address_sort: Sort,
    outside: Dynamic,
}

impl PartialHeap {
    /// `outside` is a fixed value of the frontend's value sort. It masks cells
    /// outside the owned domain before a heap is passed to an opaque predicate.
    pub fn new(values: Array, owned: Array, outside: Dynamic) -> Result<Self, Text> {
        let address_sort = values
            .get_sort()
            .array_domain()
            .ok_or_else(|| Text::from("heap values must be an array"))?;
        if values.get_sort().array_range() != Some(outside.get_sort())
            || owned.get_sort() != Sort::array(&address_sort, &Sort::bool())
        {
            return Err(Text::from(
                "incompatible partial-heap value or ownership sorts",
            ));
        }
        Ok(Self {
            values,
            owned,
            address_sort,
            outside,
        })
    }

    pub fn values(&self) -> &Array {
        &self.values
    }
    pub fn owned(&self) -> &Array {
        &self.owned
    }

    pub fn fresh_like(&self, prefix: &str) -> Self {
        Self {
            values: Array::fresh_const(prefix, &self.address_sort, &self.outside.get_sort()),
            owned: Array::fresh_const(prefix, &self.address_sort, &Sort::bool()),
            address_sort: self.address_sort.clone(),
            outside: self.outside.clone(),
        }
    }

    pub fn fresh_address(&self) -> Dynamic {
        Dynamic::fresh_const("heap_address", &self.address_sort)
    }

    fn check_address(&self, address: &Dynamic) -> Result<(), Text> {
        if address.get_sort() == self.address_sort {
            Ok(())
        } else {
            Err(Text::from("heap address has the wrong sort"))
        }
    }

    pub fn load(&self, address: &Dynamic) -> Result<Dynamic, Text> {
        self.check_address(address)?;
        Ok(self.values.select(address))
    }

    pub fn owns(&self, address: &Dynamic) -> Result<Bool, Text> {
        self.check_address(address)?;
        Ok(self
            .owned
            .select(address)
            .as_bool()
            .expect("validated ownership sort"))
    }

    /// Functional update. The caller separately proves ownership for a write,
    /// or freshness for allocation; this operation does not assume either.
    pub fn store(&self, address: &Dynamic, value: &Dynamic) -> Result<Self, Text> {
        self.check_address(address)?;
        if value.get_sort() != self.outside.get_sort() {
            return Err(Text::from("heap value has the wrong sort"));
        }
        Ok(Self {
            values: self.values.store(address, value),
            ..self.clone()
        })
    }

    pub fn set_owned(&self, address: &Dynamic, owned: bool) -> Result<Self, Text> {
        self.check_address(address)?;
        Ok(Self {
            owned: self.owned.store(address, &Bool::from_bool(owned)),
            ..self.clone()
        })
    }

    pub fn empty(&self) -> Bool {
        self.owned.eq(&Array::const_array(
            &self.address_sort,
            &Bool::from_bool(false),
        ))
    }

    pub fn points_to(&self, address: &Dynamic, value: &Dynamic) -> Result<Bool, Text> {
        let read = self.load(address)?;
        if value.get_sort() != self.outside.get_sort() {
            return Err(Text::from("points-to value has the wrong sort"));
        }
        let singleton = Array::const_array(&self.address_sort, &Bool::from_bool(false))
            .store(address, &Bool::from_bool(true));
        Ok(Bool::and(&[self.owned.eq(&singleton), read.eq(value)]))
    }

    /// Existentially split only the ownership domain. Sharing the total value
    /// array preserves every owned value without constraining unowned cells.
    pub fn separate<E>(
        &self,
        left: impl FnOnce(&Self) -> Result<Bool, E>,
        right: impl FnOnce(&Self) -> Result<Bool, E>,
    ) -> Result<Bool, E> {
        let a = Self {
            owned: Array::fresh_const("left_domain", &self.address_sort, &Sort::bool()),
            ..self.clone()
        };
        let b = Self {
            owned: Array::fresh_const("right_domain", &self.address_sort, &Sort::bool()),
            ..self.clone()
        };
        let p = left(&a)?;
        let q = right(&b)?;
        let address = self.fresh_address();
        let av = a.owned.select(&address).as_bool().expect("Boolean domain");
        let bv = b.owned.select(&address).as_bool().expect("Boolean domain");
        let partition = Bool::and(&[
            self.owned.select(&address).eq(Bool::or(&[&av, &bv])),
            Bool::and(&[&av, &bv]).not(),
        ]);
        let partition = z3::ast::forall_const(&[&address], &[], &partition);
        Ok(z3::ast::exists_const(
            &[&a.owned, &b.owned],
            &[],
            &Bool::and(&[p, q, partition]),
        ))
    }

    /// All disjoint extensions satisfying the antecedent must make the union
    /// satisfy the consequent. A same-heap implication is not a magic wand.
    pub fn wand<E>(
        &self,
        left: impl FnOnce(&Self) -> Result<Bool, E>,
        right: impl FnOnce(&Self) -> Result<Bool, E>,
    ) -> Result<Bool, E> {
        let frame = self.fresh_like("wand_frame");
        let union = self.fresh_like("wand_union");
        let p = left(&frame)?;
        let q = right(&union)?;
        let address = self.fresh_address();
        let a = self
            .owned
            .select(&address)
            .as_bool()
            .expect("Boolean domain");
        let b = frame
            .owned
            .select(&address)
            .as_bool()
            .expect("Boolean domain");
        let composition = Bool::and(&[
            Bool::and(&[&a, &b]).not(),
            union.owned.select(&address).eq(Bool::or(&[&a, &b])),
            a.implies(
                union
                    .values
                    .select(&address)
                    .eq(self.values.select(&address)),
            ),
            b.implies(
                union
                    .values
                    .select(&address)
                    .eq(frame.values.select(&address)),
            ),
        ]);
        let composition = z3::ast::forall_const(&[&address], &[], &composition);
        Ok(z3::ast::forall_const(
            &[&frame.values, &frame.owned, &union.values, &union.owned],
            &[],
            &Bool::and(&[p, composition]).implies(q),
        ))
    }

    pub fn relation(&self, name: &str, args: &[Dynamic]) -> Bool {
        let restricted = self.canonical_values();
        let mut values: List<Dynamic> = [
            Dynamic::from_ast(&restricted),
            Dynamic::from_ast(&self.owned),
        ]
        .into_iter()
        .collect();
        values.extend(args.iter().cloned());
        let sorts: List<Sort> = values.iter().map(Ast::get_sort).collect();
        let sorts: List<&Sort> = sorts.iter().collect();
        let args: List<&dyn Ast> = values.iter().map(|a| a as &dyn Ast).collect();
        z3::FuncDecl::new(
            format!("spatial:{}:{}:{}", name.len(), name, args.len() - 2),
            &sorts,
            &Sort::bool(),
        )
        .apply(&args)
        .as_bool()
        .expect("Boolean spatial relation")
    }

    fn canonical_values(&self) -> Array {
        let outside = Array::const_array(&self.address_sort, &self.outside);
        let condition = Bool::fresh_const("heap_mask");
        let value = Dynamic::fresh_const("heap_value", &self.outside.get_sort());
        let choice = condition.ite(&value, &self.outside);
        let context = self.values.get_ctx();
        let arrays = [
            self.owned.get_z3_ast(),
            self.values.get_z3_ast(),
            outside.get_z3_ast(),
        ];
        // SAFETY: construction validates both array domains and ranges. The
        // built-in ite is (Bool, Value, Value) -> Value, and all nodes belong to
        // the active Z3 context. Mapping it preserves the array's value sort.
        unsafe {
            let ctx = context.get_z3_context();
            let app = z3_sys::Z3_to_app(ctx, choice.get_z3_ast()).expect("ite application");
            let declaration = z3_sys::Z3_get_app_decl(ctx, app).expect("ite declaration");
            let mapped = z3_sys::Z3_mk_map(ctx, declaration, 3, arrays.as_ptr())
                .expect("validated heap restriction");
            Array::wrap(context, mapped)
        }
    }
}
