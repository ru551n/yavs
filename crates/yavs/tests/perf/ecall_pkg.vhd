-- Pure functions called while elaborating (ecall_tb.vhd): their results are cached per closure.
package ecall_pkg is
  constant K : integer := 7;
  function f (x : integer) return integer;
end;
package body ecall_pkg is
  function f (x : integer) return integer is
    variable acc : integer := 0;
  begin
    for i in 1 to x loop acc := acc + i * K; end loop;
    return acc + 1;
  end;
end;
