-- Foreign subprograms of native.c. The foreign attribute binds each one to the loaded library;
-- the bodies are only there for tools that want one and never run.
package native_pkg is
  subtype chunk_t is string(1 to 64);

  impure function native_add1(x : integer) return integer;
  attribute foreign of native_add1 : function is "VHPIDIRECT native_add1";
  impure function py_exec(chunk : chunk_t; length : integer) return integer;
  attribute foreign of py_exec : function is "VHPIDIRECT py_exec";
  impure function py_eval_int(chunk : chunk_t; length : integer) return integer;
  attribute foreign of py_eval_int : function is "VHPIDIRECT py_eval_int";

  -- Python source or an expression of any length up to the chunk
  impure function exec(source : string) return integer;
  impure function eval(expr : string) return integer;
end package;

package body native_pkg is
  impure function native_add1(x : integer) return integer is begin return 0; end;
  impure function py_exec(chunk : chunk_t; length : integer) return integer is begin return 0; end;
  impure function py_eval_int(chunk : chunk_t; length : integer) return integer is begin return 0; end;

  impure function exec(source : string) return integer is
    variable chunk : chunk_t := (others => ' ');
  begin
    chunk(1 to source'length) := source;
    return py_exec(chunk, source'length);
  end;

  impure function eval(expr : string) return integer is
    variable chunk : chunk_t := (others => ' ');
  begin
    chunk(1 to expr'length) := expr;
    return py_eval_int(chunk, expr'length);
  end;
end package body;
