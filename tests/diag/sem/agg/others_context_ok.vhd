-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.3.3
-- `others` aggregates in the other constrained contexts: result expressions, actuals
-- and defaults of constrained formals (procedure calls share build_call), qualified
-- expressions, nested aggregates.
package p is
  subtype w8 is bit_vector(7 downto 0);
  type arr2 is array (0 to 1) of w8;
  function take (x : in w8) return w8;
  function ret (n : natural) return w8;
  function ret2 (n : natural) return w8;
  function g (x : w8 := (others => '1')) return natural;
end package;
package body p is
  function take (x : in w8) return w8 is begin return x; end function;
  function ret (n : natural) return w8 is
  begin
    return (others => '0');
  end function;
  function ret2 (n : natural) return w8 is
  begin
    if n = 0 then return (others => '1'); else return (others => '0'); end if;
  end function;
  function g (x : w8 := (others => '1')) return natural is

  begin
    return 0;
  end function;
  procedure pr (n : natural) is
    variable v : w8;
    variable a : arr2 := (others => (others => '0'));
    constant c : w8 := w8'(others => '1');
  begin
    v := take((others => '0'));
    v := take(x => (others => '1'));
    v := w8'(others => '0');
    a := (others => (others => '1'));
    a(0) := (others => '1');
    a := ((others => '0'), (others => '1'));
    if v = w8'(others => '0') then null; end if;
  end procedure;
end package body;
