//! Runtime values, variables and arrays.

use std::rc::Rc;

/// AMOS strings are byte strings (ISO-8859-1), immutable and shared.
pub type AStr = Rc<[u8]>;

/// Maximum string length (`String_Max`).
pub const STRING_MAX: usize = 0xFFC0;

/// The "omitted parameter" sentinel (`EntNul`), e.g. the x of `At(,10)`.
pub const ENT_NUL: i32 = i32::MIN;

pub fn astr(s: &[u8]) -> AStr {
    Rc::from(s)
}

pub fn empty_str() -> AStr {
    thread_local! {
        static EMPTY: AStr = Rc::from(&b""[..]);
    }
    EMPTY.with(|e| e.clone())
}

#[derive(Clone, Debug)]
pub enum Value {
    Int(i32),
    /// Float. In single precision mode the value is always exactly
    /// representable as Motorola FFP.
    Float(f64),
    Str(AStr),
}

impl Default for Value {
    fn default() -> Self {
        Value::Int(0)
    }
}

impl Value {
    pub fn str(s: &[u8]) -> Value {
        Value::Str(astr(s))
    }

    pub fn is_str(&self) -> bool {
        matches!(self, Value::Str(_))
    }

    pub fn type_code(&self) -> u8 {
        match self {
            Value::Int(_) => 0,
            Value::Float(_) => 1,
            Value::Str(_) => 2,
        }
    }

    /// The default (cleared) value of a variable of `ty` (0 int, 1 float, 2 string).
    pub fn zero(ty: u8) -> Value {
        match ty {
            1 => Value::Float(0.0),
            2 => Value::Str(empty_str()),
            _ => Value::Int(0),
        }
    }
}

/// Element storage of an array.
#[derive(Clone, Debug)]
pub enum ArrayData {
    Int(Vec<i32>),
    Float(Vec<f64>),
    Str(Vec<AStr>),
}

#[derive(Clone, Debug)]
pub struct Array {
    /// Maximum index of every dimension (`Dim A(10)` gives 10).
    pub dims: Vec<u16>,
    pub data: ArrayData,
}

impl Array {
    pub fn new(ty: u8, dims: Vec<u16>) -> Array {
        let n: usize = dims.iter().map(|&d| d as usize + 1).product();
        let data = match ty {
            1 => ArrayData::Float(vec![0.0; n]),
            2 => ArrayData::Str(vec![empty_str(); n]),
            _ => ArrayData::Int(vec![0; n]),
        };
        Array { dims, data }
    }

    pub fn len(&self) -> usize {
        match &self.data {
            ArrayData::Int(v) => v.len(),
            ArrayData::Float(v) => v.len(),
            ArrayData::Str(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Flat index of an element; `None` if an index is out of range.
    pub fn index(&self, idx: &[i32]) -> Option<usize> {
        if idx.len() != self.dims.len() {
            return None;
        }
        let mut flat = 0usize;
        for (&i, &max) in idx.iter().zip(&self.dims) {
            if i < 0 || i as u32 > max as u32 {
                return None;
            }
            flat = flat * (max as usize + 1) + i as usize;
        }
        Some(flat)
    }

    pub fn get(&self, i: usize) -> Value {
        match &self.data {
            ArrayData::Int(v) => Value::Int(v[i]),
            ArrayData::Float(v) => Value::Float(v[i]),
            ArrayData::Str(v) => Value::Str(v[i].clone()),
        }
    }

    pub fn set(&mut self, i: usize, val: Value) {
        match (&mut self.data, val) {
            (ArrayData::Int(v), Value::Int(x)) => v[i] = x,
            (ArrayData::Int(v), Value::Float(x)) => v[i] = float_to_int(x),
            (ArrayData::Float(v), Value::Float(x)) => v[i] = x,
            (ArrayData::Float(v), Value::Int(x)) => v[i] = x as f64,
            (ArrayData::Str(v), Value::Str(s)) => v[i] = s,
            _ => {}
        }
    }
}

/// A variable slot.
#[derive(Clone, Debug, Default)]
pub enum Var {
    /// Never assigned (reads as 0 / "").
    #[default]
    Unset,
    Scalar(Value),
    Array(Box<Array>),
    /// `Def Fn` definition: position of its parameter list in the code.
    Fn(usize),
}

/// Float to integer conversion: truncation toward zero with saturation
/// (behaviour of `SPFix` / `IEEEDPFix`).
pub fn float_to_int(x: f64) -> i32 {
    if x.is_nan() {
        0
    } else if x >= i32::MAX as f64 {
        i32::MAX
    } else if x <= i32::MIN as f64 {
        i32::MIN
    } else {
        x.trunc() as i32
    }
}
