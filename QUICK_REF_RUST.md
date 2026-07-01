# **🦀 QUICK_REF_RUST.md — Cheat Sheet for Compiler Idioms**

**Keep this bookmark open while coding. Paste patterns as-is.**

---

## **Pattern 1: Enum + Pattern Matching (AST)**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "nodeType")]
pub enum ASTNode {
    #[serde(rename = "function_def")]
    FunctionDef { name: String, body: Box<ASTNode> },
    
    #[serde(rename = "call")]
    Call { function: Box<ASTNode>, arguments: Vec<ASTNode> },
}

// Usage:
match node {
    ASTNode::FunctionDef { name, body } => { /* ... */ }
    ASTNode::Call { function, arguments } => { /* ... */ }
}
```

---

## **Pattern 2: Result<T, E> Error Handling**

```rust
// Function signature:
pub fn parse_expr(&mut self) -> Result<ASTNode, String> {
    // Success:
    Ok(ASTNode::Literal { ... })
    
    // Error:
    Err("Expected number".to_string())
}

// Usage with ?:
let expr = self.parse_expr()?;  // Early return on Err

// Usage with match:
match self.parse_expr() {
    Ok(expr) => println!("Success: {:?}", expr),
    Err(e) => eprintln!("Error: {}", e),
}
```

---

## **Pattern 3: HashMap for Symbol Tables**

```rust
use std::collections::HashMap;

let mut environment: HashMap<String, Type> = HashMap::new();

// Insert
environment.insert("x".to_string(), Type::Int64);

// Look up
if let Some(ty) = environment.get("x") {
    println!("Type of x: {:?}", ty);
} else {
    println!("x not found");
}

// Update
environment.entry("x".to_string())
    .or_insert(Type::Int64)
    .clone();
```

---

## **Pattern 4: Vec for Stack (Scopes)**

```rust
struct Analyzer {
    scopes: Vec<HashMap<String, String>>,
}

impl Analyzer {
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    
    fn declare(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), "defined".to_string());
        }
    }
    
    fn is_declared(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains_key(name))
    }
}
```

---

## **Pattern 5: String vs &str**

```rust
// Owned String (use in structs, HashMap keys, return values)
pub struct Token {
    pub value: String,  // ✓
}

// Borrowed &str (use in function parameters)
pub fn parse(&mut self, input: &str) {  // ✓
    let owned = input.to_string();  // &str → String
}

// Convert:
let s = "hello";           // &str
let owned = s.to_string(); // → String
let borrowed = &owned;     // String → &str
```

---

## **Pattern 6: Box<T> for Recursive Types**

```rust
// ✗ Won't compile (infinite size):
pub enum Expr {
    Call { func: Expr, args: Vec<Expr> },
}

// ✓ Correct (pointer indirection):
pub enum Expr {
    Call { 
        func: Box<Expr>, 
        args: Vec<Expr> 
    },
}

// Usage:
let call = Expr::Call {
    func: Box::new(Expr::Var { name: "foo".into() }),
    args: vec![],
};
```

---

## **Pattern 7: Serde JSON Serialization**

```rust
use serde::{Serialize, Deserialize};
use serde_json;

#[derive(Serialize, Deserialize, Debug)]
pub struct Data {
    pub name: String,
    pub value: i64,
}

// Serialize:
let obj = Data { name: "test".into(), value: 42 };
let json = serde_json::to_string_pretty(&obj)?;
std::fs::write("file.json", json)?;

// Deserialize:
let json = std::fs::read_to_string("file.json")?;
let obj: Data = serde_json::from_str(&json)?;
```

---

## **Pattern 8: Custom Serde Field Names**

```rust
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "nodeType")]
    pub kind: String,
    
    #[serde(rename = "line", skip_serializing_if = "Option::is_none")]
    pub line_number: Option<usize>,
}

// JSON:
// { "nodeType": "call", "line": 5 }
```

---

## **Pattern 9: Vec Clone vs Iter**

```rust
// ✗ Inefficient (clones entire vec):
for item in vec.clone() { }

// ✓ Borrow (no clone):
for item in &vec { }
for item in vec.iter() { }

// ✓ Consume (moves ownership):
for item in vec { }

// Collect into new vec:
let new_vec: Vec<_> = vec.iter().map(|x| x + 1).collect();
```

---

## **Pattern 10: Option<T> (Null-safety)**

```rust
// Instead of NULL pointers:
pub fn find_type(&self, name: &str) -> Option<Type> {
    self.types.get(name).cloned()
}

// Usage:
match find_type("x") {
    Some(ty) => println!("Found: {:?}", ty),
    None => println!("Not found"),
}

// Or with if-let:
if let Some(ty) = find_type("x") {
    println!("Found: {:?}", ty);
}

// Unwrap (panics if None, use only in tests):
let ty = find_type("x").unwrap();
```

---

## **Pattern 11: Trait Methods with self**

```rust
pub trait Visitor {
    fn visit(&self, node: &ASTNode) -> Result<(), String>;
}

pub struct TypeChecker;

impl Visitor for TypeChecker {
    fn visit(&self, node: &ASTNode) -> Result<(), String> {
        match node {
            ASTNode::FunctionDef { .. } => { /* ... */ }
            _ => Ok(()),
        }
    }
}

// Usage:
let checker = TypeChecker;
checker.visit(&ast)?;
```

---

## **Pattern 12: Tuple Destructuring**

```rust
// Parameters as Vec of tuples:
let parameters: Vec<(String, Option<String>)> = vec![
    ("x".into(), Some("i64".into())),
    ("y".into(), None),
];

// Iterate with destructuring:
for (name, type_opt) in &parameters {
    println!("Param: {} : {:?}", name, type_opt);
}

// Extract:
if let Some((name, ty)) = parameters.first() {
    println!("{}: {:?}", name, ty);
}
```

---

## **Pattern 13: Closure with Capture**

```rust
let threshold = 10;

// Closure captures 'threshold' by reference
let check = |x| x > threshold;

println!("{}", check(15));  // true
println!("{}", check(5));   // false

// Capture by value (move):
let values = vec![1, 2, 3];
let process = move || println!("{:?}", values);  // Owns values
process();
// println!("{:?}", values);  // ✗ Error: values moved
```

---

## **Pattern 14: Static Mutable Counter (Unsafe)**

```rust
static mut COUNTER: usize = 0;

pub fn next_id() -> usize {
    unsafe {
        let id = COUNTER;
        COUNTER += 1;
        id
    }
}

// ⚠️ Use sparingly, only for counters/IDs
```

---

## **Pattern 15: String Interpolation**

```rust
let name = "Carp";
let version = 1;

// format!() → String
let msg = format!("Compiler {} v{}", name, version);

// println!() → stdout
println!("Building {} v{}", name, version);

// eprintln!() → stderr (errors)
eprintln!("Error in file: {}", filename);
```

---

## **Pattern 16: Clone vs Copy**

```rust
// Primitives auto-Copy (cheap):
let x = 5;
let y = x;  // ✓ OK, x still usable

// Complex types must Clone:
let s = "hello".to_string();
let s2 = s.clone();  // ✓ Explicit clone
// println!("{}", s);  // ✗ Error: s moved

// In structs, derive Clone:
#[derive(Clone)]
pub struct Node { ... }

let node2 = node.clone();  // ✓ OK
```

---

## **Pattern 17: Lifetime Annotations (References)**

```rust
// Simple: references from same input
pub fn longest<'a>(s1: &'a str, s2: &'a str) -> &'a str {
    if s1.len() > s2.len() { s1 } else { s2 }
}

// Most functions don't need explicit lifetimes (Rust elides them):
pub fn parse(&mut self, input: &str) -> Result<AST, String> {
    // ✓ Implicitly: &'mut self, &str, Result<T, E>
}
```

---

## **Pattern 18: Command-Line Arguments**

```rust
use std::env;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    
    if args.len() != 2 {
        eprintln!("Usage: {} <file.carp>", args[0]);
        std::process::exit(1);
    }
    
    let filename = &args[1];
    let content = fs::read_to_string(filename)?;
    
    // Process content...
    
    Ok(())
}
```

---

## **Pattern 19: File I/O**

```rust
use std::fs;

// Read entire file:
let content = fs::read_to_string("input.txt")?;

// Write entire file:
fs::write("output.txt", "Hello, world!")?;

// Append:
use std::fs::OpenOptions;
use std::io::Write;

let mut file = OpenOptions::new()
    .append(true)
    .open("log.txt")?;
file.write_all(b"New line\n")?;
```

---

## **Pattern 20: Custom Debug Formatting**

```rust
use std::fmt;

pub struct Token {
    pub kind: String,
    pub value: String,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({}, {})", self.kind, self.value)
    }
}

// Automatic with #[derive(Debug)]:
#[derive(Debug)]
pub struct AST { ... }
```

---

## **Idioms: DO's and DON'Ts**

### **✓ DO**

```rust
// 1. Return Result for fallible operations
pub fn parse() -> Result<AST, String> { }

// 2. Use &T for non-owning borrows
pub fn process(&self, input: &str) { }

// 3. Use ? operator for error propagation
let value = risky_operation()?;

// 4. Destructure in match
match node {
    ASTNode::Var { name } => { }
}

// 5. Use closure for callbacks
vec.iter().filter(|x| x > 10)
```

### **✗ DON'T**

```rust
// 1. Don't unwrap in library code
let val = option.unwrap();  // Panics!

// 2. Don't clone unnecessarily
let copy = original.clone();  // Use &original instead

// 3. Don't use mutable statics
static mut GLOBAL: i32 = 0;  // Avoid

// 4. Don't catch all errors
match result {
    Ok(_) => { }
    Err(_) => { }  // Lost error info!
}

// 5. Don't ignore Result
some_function();  // ✗ Returns Result, ignores it
some_function()?;  // ✓ Handles error
```

---

## **Debugging**

```rust
// Print entire structure
println!("{:#?}", node);  // Pretty-print Debug

// Inline assertions
assert_eq!(1 + 1, 2, "Math is broken");

// Unreachable code
unreachable!("This should never execute");

// Panic for debugging
panic!("Debug point: {:?}", value);

// dbg! macro
let x = dbg!(expensive_computation());
```

---

## **Build & Test**

```bash
# Build
cargo build --release

# Run tests
cargo test

# Run with backtrace
RUST_BACKTRACE=1 cargo run

# Check for issues without building
cargo clippy

# Format code
cargo fmt
```

---

## **Resource Limits (Optimization)**

```rust
// Preallocate vectors
let mut v = Vec::with_capacity(1000);

// Use references in collections (avoid clones)
let refs: Vec<&Item> = items.iter().collect();

// Use Cow for conditional ownership
use std::borrow::Cow;
let s: Cow<str> = if condition {
    Cow::Owned("allocated".to_string())
} else {
    Cow::Borrowed("static")
};
```

---

## **Common Errors & Fixes**

| Error | Fix |
|-------|-----|
| "borrow of moved value" | Use `&` or `.clone()` |
| "expected `&str`, found `String`" | Use `&s` or `s.as_str()` |
| "trait object without lifetime" | Add `dyn Trait + 'a` |
| "expected usize, found i64" | Use `.len() as i64` |
| "no method named `push`" | Call on `&mut Vec` |

---

## **Useful Crates (Already in Workspace)**

```toml
serde_json     # JSON parsing/generating
serde          # Serialization framework
regex          # Regular expressions (optional)
```

---

**Print this, keep nearby, copy patterns as-is!** 🦀

¡Éxito con Rust!
