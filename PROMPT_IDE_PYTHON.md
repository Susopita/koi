# **🐍 PROMPT IDE: Python Textual TUI (Fase 2)**

**Responsabilidad:** Implementar IDE en **Python Textual** que llama compilador Rust vía subprocess.

**Inicio:** Después de Fase 1 (koi complete)

**Tecnología:** Python 3.10+, Textual TUI framework

**No es obligatorio para MVP, pero es bonus**

---

## **Estructura**

```
pond/
├── main.py                # Entry point
├── ui.py                  # Textual UI components
├── compiler.py            # Subprocess wrapper
├── syntax_highlighter.py  # Syntax coloring
├── requirements.txt
└── README.md
```

**requirements.txt:**
```
textual>=0.40.0
pygments>=2.16.0
```

---

## **Architecture: IDE ↔ Compiler IPC**

```
┌─────────────────────┐
│  Python Textual UI  │
└──────────┬──────────┘
           │ 1. Write file
           ↓
    /tmp/test.carp
           │ 2. Spawn subprocess
           ↓
┌──────────────────────────┐
│  ./target/release/koi-ast│
│  test.carp               │
│  → /tmp/ast.json         │
└──────────┬───────────────┘
           │
    /tmp/ast.json
           │
┌──────────────────────────┐
│  ./target/release/koi-ir │
│  → /tmp/ir.json          │
└──────────┬───────────────┘
           │
    /tmp/ir.json
           │
┌──────────────────────────┐
│  ./target/release/koi-   |
│  assembly                │
│  → output.s              │
└──────────┬───────────────┘
           │
     output.s
           │
    gcc → output (binary)
           │
     Display result in IDE
```

---

## **Parte 1: Compiler Wrapper (compiler.py)**

```python
# pond/compiler.py

import subprocess
import json
import os
from pathlib import Path
from typing import Optional, Dict, Any

class CompilerError(Exception):
    pass

class CompilerWrapper:
    def __init__(self, compiler_path: str = "./target/release"):
        self.compiler_path = compiler_path
        self.tmp_dir = Path("/tmp")
        
    def compile(self, source_file: str) -> Dict[str, Any]:
        """
        Full compilation pipeline:
        input → AST → IR → Assembly → Binary
        
        Returns:
            {
                "success": bool,
                "binary": str | None,
                "assembly": str | None,
                "errors": [{"phase": str, "message": str, ...}],
                "execution": {"stdout": str, "stderr": str} | None
            }
        """
        result = {
            "success": False,
            "binary": None,
            "assembly": None,
            "errors": [],
            "execution": None,
        }
        
        try:
            # Step 1: AST (Lexing + Parsing)
            self._run_ast(source_file)
            
            # Step 2: IR (Inference + IR)
            self._run_ir()
            
            # Step 3: Assembly (Codegen)
            self._run_assembly()
            
            # Step 4: Assemble + Link
            self._assemble_and_link()
            
            result["success"] = True
            result["binary"] = "output"
            result["assembly"] = self._read_assembly()
            
            # Step 5: Execute (optional)
            result["execution"] = self._execute_binary()
            
        except CompilerError as e:
            result["errors"] = [{"phase": e.phase, "message": str(e)}]
        except Exception as e:
            result["errors"] = [{"phase": "unknown", "message": str(e)}]
        
        return result
    
    def _run_ast(self, source_file: str) -> None:
        """Run koi-ast binary"""
        try:
            result = subprocess.run(
                [f"{self.compiler_path}/koi-ast", source_file],
                capture_output=True,
                text=True,
                timeout=10,
            )
            
            if result.returncode != 0:
                raise CompilerError(
                    phase="koi-ast",
                    message=result.stderr or "koi-ast failed",
                )
            
            # Validate AST JSON
            ast_file = self.tmp_dir / "ast.json"
            if not ast_file.exists():
                raise CompilerError(
                    phase="koi-ast",
                    message="AST not generated",
                )
            
            # Try to parse as JSON
            with open(ast_file) as f:
                json.load(f)
                
        except subprocess.TimeoutExpired:
            raise CompilerError(phase="koi-ast", message="Timeout (>10s)")
    
    def _run_ir(self) -> None:
        """Run koi-ir binary"""
        try:
            result = subprocess.run(
                [f"{self.compiler_path}/koi-ir"],
                capture_output=True,
                text=True,
                timeout=10,
            )
            
            if result.returncode != 0:
                raise CompilerError(
                    phase="koi-ir",
                    message=result.stderr or "koi-ir failed",
                )
            
            ir_file = self.tmp_dir / "ir.json"
            if not ir_file.exists():
                raise CompilerError(
                    phase="koi-ir",
                    message="IR not generated",
                )
            
            with open(ir_file) as f:
                json.load(f)
                
        except subprocess.TimeoutExpired:
            raise CompilerError(phase="koi-ir", message="Timeout (>10s)")
    
    def _run_assembly(self) -> None:
        """Run koi-assembly binary"""
        try:
            result = subprocess.run(
                [f"{self.compiler_path}/koi-assembly"],
                capture_output=True,
                text=True,
                timeout=10,
            )
            
            if result.returncode != 0:
                raise CompilerError(
                    phase="koi-assembly",
                    message=result.stderr or "koi-assembly failed",
                )
            
            if not Path("/tmp/output.s").exists():
                raise CompilerError(
                    phase="koi-assembly",
                    message="Assembly not generated",
                )
                
        except subprocess.TimeoutExpired:
            raise CompilerError(phase="koi-assembly", message="Timeout (>10s)")
    
    def _assemble_and_link(self) -> None:
        """Assemble and link"""
        try:
            # Assemble
            result = subprocess.run(
                ["gcc", "-c", "output.s", "-o", "output.o"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            
            if result.returncode != 0:
                raise CompilerError(
                    phase="koi-assembly",
                    message=result.stderr or "GCC assembly failed",
                )
            
            # Link
            result = subprocess.run(
                ["gcc", "output.o", "-o", "output"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            
            if result.returncode != 0:
                raise CompilerError(
                    phase="koi-assembly",
                    message=result.stderr or "GCC linking failed",
                )
                
        except subprocess.TimeoutExpired:
            raise CompilerError(phase="koi-assembly", message="Timeout")
    
    def _read_assembly(self) -> str:
        """Read generated assembly"""
        try:
            with open("output.s") as f:
                return f.read()
        except FileNotFoundError:
            return ""
    
    def _execute_binary(self) -> Optional[Dict[str, str]]:
        """Execute compiled binary"""
        try:
            result = subprocess.run(
                ["./output"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            
            return {
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exitcode": result.returncode,
            }
        except Exception:
            return None


class CompilerError(Exception):
    def __init__(self, phase: str, message: str):
        self.phase = phase
        self.message = message
        super().__init__(f"[{phase}] {message}")
```

---

## **Parte 2: Syntax Highlighter (syntax_highlighter.py)**

```python
# pond/syntax_highlighter.py
s
from pygments.lexer import RegexLexer
from pygments.token import (
    Text, Comment, Operator, Keyword, Name,
    String, Number, Whitespace, Literal
)

class CarpLexer(RegexLexer):
    """Syntax highlighter for Carp language"""
    
    name = 'Carp'
    aliases = ['carp']
    filenames = ['*.carp']
    
    tokens = {
        'root': [
            # Whitespace
            (r'\s+', Whitespace),
            
            # Comments
            (r';.*?$', Comment.Single),
            
            # Keywords
            (r'\b(defn|defstruct|lambda|let|if|loop|do|new|match)\b',
             Keyword),
            
            # Built-in functions
            (r'\b(print|printf|malloc|free|new|delete)\b', Name.Builtin),
            
            # Numbers
            (r'-?\d+\.\d+', Number.Float),
            (r'-?\d+', Number.Integer),
            
            # Strings
            (r'"(?:\\.|[^"\\])*"', String),
            
            # Operators
            (r'[+\-*/%=<>!&|]+', Operator),
            
            # Symbols and identifiers
            (r'[a-zA-Z_][a-zA-Z0-9_?!]*', Name),
            
            # Delimiters
            (r'[\(\)\[\]\{\}]', Literal),
            
            # Punctuation
            (r'[:,]', Text),
        ],
    }
```

---

## **Parte 3: Textual UI (ui.py)**

```python
# pond/ui.py

from textual.app import ComposeResult
from textual.containers import Container, Vertical, Horizontal
from textual.widgets import (
    Header, Footer, Static, TextArea,
    Button, Label, RichLog
)
from textual.binding import Binding
from compiler import CompilerWrapper
import json

class CarpIDE(ComposeResult):
    """Main IDE layout with editor, output, status"""
    
    BINDINGS = [
        Binding("ctrl+s", "save", "Save"),
        Binding("ctrl+b", "build", "Build"),
        Binding("ctrl+r", "run", "Run"),
        Binding("ctrl+q", "quit", "Quit"),
    ]
    
    def compose(self) -> ComposeResult:
        yield Header()
        
        with Horizontal():
            # Left: Editor
            with Vertical(id="left-panel"):
                yield Label("📝 Editor", id="editor-title")
                yield TextArea(id="editor", language="carp")
            
            # Right: Output
            with Vertical(id="right-panel"):
                yield Label("📋 Output", id="output-title")
                yield RichLog(id="output-log", highlight=True)
        
        with Horizontal(id="bottom-panel"):
            yield Button("Build", id="build-btn", variant="primary")
            yield Button("Run", id="run-btn")
            yield Label("Ready", id="status")
        
        yield Footer()
    
    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "build-btn":
            self.action_build()
        elif event.button.id == "run-btn":
            self.action_run()
    
    def action_build(self) -> None:
        """Build the current program"""
        editor = self.query_one("#editor", TextArea)
        source = editor.text
        
        # Write to temp file
        with open("/tmp/test.carp", "w") as f:
            f.write(source)
        
        # Update status
        status = self.query_one("#status", Label)
        status.update("Building...")
        
        # Compile
        compiler = CompilerWrapper()
        result = compiler.compile("/tmp/test.carp")
        
        # Update output
        output = self.query_one("#output-log", RichLog)
        output.clear()
        
        if result["success"]:
            output.write("✓ Build successful!")
            if result["assembly"]:
                output.write("\n[Assembly]\n")
                output.write(result["assembly"][:500] + "...")
            status.update("Built ✓")
        else:
            output.write("✗ Build failed\n\n")
            for error in result["errors"]:
                output.write(f"[{error['phase']}] {error['message']}\n")
            status.update("Build failed ✗")
    
    def action_run(self) -> None:
        """Run the compiled binary"""
        compiler = CompilerWrapper()
        result = compiler.compile("/tmp/test.carp")
        
        output = self.query_one("#output-log", RichLog)
        output.clear()
        
        if result["success"] and result["execution"]:
            output.write("✓ Execution:\n\n")
            if result["execution"]["stdout"]:
                output.write(result["execution"]["stdout"])
            if result["execution"]["stderr"]:
                output.write(f"\n[stderr]\n{result['execution']['stderr']}")
        else:
            output.write("✗ Cannot run (build first)")
    
    def action_save(self) -> None:
        """Save to file"""
        editor = self.query_one("#editor", TextArea)
        with open("current.carp", "w") as f:
            f.write(editor.text)
        
        status = self.query_one("#status", Label)
        status.update("Saved ✓")
```

---

## **Parte 4: Main Entry Point (main.py)**

```python
#!/usr/bin/env python3
# pond/main.py

from textual.app import App, ComposeResult
from textual.widgets import Header, Footer
from textual.containers import Container
from ui import CarpIDE

class CarpIDEApp(App):
    """Main application"""
    
    TITLE = "Pond - IDE para Carp"
    SUBTITLE = "Rust · Type-Safe · x86-64"
    
    CSS = """
    Screen {
        layout: vertical;
    }
    
    #left-panel {
        width: 1fr;
        border: solid $accent;
    }
    
    #right-panel {
        width: 1fr;
        border: solid $accent;
    }
    
    #bottom-panel {
        height: 3;
        border-top: solid $accent;
        background: $surface;
    }
    
    #editor-title, #output-title {
        width: 1fr;
        background: $boost;
        color: $text;
        padding: 0 1;
    }
    
    Button {
        margin-right: 1;
    }
    
    #status {
        dock: right;
    }
    """
    
    def compose(self) -> ComposeResult:
        yield Header()
        yield CarpIDE()
        yield Footer()

if __name__ == "__main__":
    app = CarpIDEApp()
    app.run()
```

---

## **Instalación & Uso**

```bash
# Setup venv
python3 -m venv venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Run IDE
python3 main.py
```

---

## **Features (MVP)**

✅ **Implemented in Phase 2:**
- [ ] Editor with syntax highlighting
- [ ] Build button → runs Rust compiler
- [ ] Run button → executes binary
- [ ] Output panel (errors + execution)
- [ ] Status bar
- [ ] Save current file

⭐ **Bonus (if time):**
- [ ] Line-by-line error highlighting
- [ ] AST/IR visualization
- [ ] Register allocator graph
- [ ] Benchmark comparison chart
- [ ] Dark/light theme toggle

---

## **Checklist Phase 2 (IDE)**

- [ ] Python Textual installed
- [ ] compiler.py works (test with manual file)
- [ ] UI renders without crashes
- [ ] Build button calls Rust compiler
- [ ] Output displayed in log
- [ ] Run button executes binary
- [ ] Syntax highlighting works
- [ ] Save functionality

---

## **Debugging IDE**

**If Rust compiler not found:**
```python
# Download the compiler in that path
self.compiler_path = "~/.pond/koi/target/release"
```

**If subprocess timeout:**
```python
# Increase timeout in compiler.py:
timeout=30,  # Was 10
```

**View raw logs:**
```bash
# Run with debug output
python3 main.py --debug
```

---

¡Pond con Python Textual! 🐍
