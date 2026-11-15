-- difftest: top=slice_target_bound_signal std=2008 ref=hand ignore=order
-- ref=hand: a signal slice target whose bound is an impure call, with values that take their
-- bounds from the target (aggregates, a string literal): the name is evaluated once (LRM
-- 10.5.2.1), so the call runs once per assignment in every form (#1157). sim2 gives these lines;
-- one reference simulator runs the call once per use (2 to 10 times). ignore=order: the two
-- concurrent assignments report at time 0 next to the process.
library ieee;
use ieee.std_logic_1164.all;

entity slice_target_bound_signal is
end entity;

architecture sim of slice_target_bound_signal is
  type iv is array (natural range <>) of integer;
  type rec is record
    a : integer;
    b : iv(0 to 1);
  end record;
  type ra is array (natural range <>) of rec;
  type ivv is array (natural range <>) of iv;
  type frec is record
    f : iv(0 to 3);
    g : integer;
  end record;
  type fixed2 is array (0 to 1) of iv(0 to 3);
  type ivs is array (natural range <>) of iv(0 to 3);
  function resv(x : ivs) return iv is
  begin
    if x'length = 0 then
      return (0 to 3 => -1);
    end if;
    return x(x'left);
  end function;
  subtype rv is resv iv(0 to 3);
  signal s, s2, s3, s4, sf, sa : iv(0 to 3) := (others => 0);
  signal l : std_ulogic_vector(0 to 3) := (others => '0');
  signal r : frec := ((others => 0), 0);
  signal a : fixed2 := (others => (others => 0));
  signal x, xf : ra(0 to 3) := (others => (0, (0, 0)));
  signal y : ivv(0 to 3)(0 to 1) := (others => (0, 0));
  signal gs : rv register := (others => 0);
  signal c, en : boolean := true;
  signal sel : integer := 1;
  alias al : iv(0 to 3) is sa;
  impure function tc(k : natural; what : string) return natural is
  begin
    report what & " bound";
    return k;
  end function;
begin
  s3(tc(1, "concurrent") to 2) <= (5, 6) when c else (7, 8);

  b : block (en) is
  begin
    gs(tc(1, "guarded") to 2) <= guarded (5, 6);
  end block;

  process is
    variable n : natural := 0;
    variable w : iv(0 to 3) := (9, 8, 7, 6);
    impure function t(k : natural) return natural is
    begin
      n := n + 1;
      return k;
    end function;
    procedure p(signal o : out iv) is
    begin
      o(t(1) to 2) <= (1, 2);
    end procedure;
  begin
    n := 0; s(t(1) to 2) <= (5, 6);
    report "positional: " & integer'image(n);
    n := 0; s(t(0) to 3) <= (others => 7);
    report "others: " & integer'image(n);
    n := 0; s(t(1) to 2) <= (1 => 5, 2 => 6);
    report "named: " & integer'image(n);
    n := 0; s(t(1) to 2) <= (1 to 2 => 7);
    report "range choice: " & integer'image(n);
    n := 0; s(t(1) to 2) <= (w(0 to 0), 6);
    report "pieces: " & integer'image(n);
    n := 0; s(t(1) to 2) <= (5, 6), (7, 8) after 1 ns, (others => 9) after 2 ns;
    report "three elements: " & integer'image(n);
    n := 0; s(t(1) to 2) <= transport (5, 6) after 3 ns;
    report "transport: " & integer'image(n);
    n := 0; s(t(1) to 2) <= (5, 6) when c else (7, 8);
    report "conditional: " & integer'image(n);
    n := 0;
    with sel select s2(t(1) to 2) <= (5, 6) when 1, (7, 8) when others;
    report "selected: " & integer'image(n);
    n := 0; l(t(1) to 2) <= "01";
    report "logic string: " & integer'image(n);
    n := 0; l(t(0) to 3) <= (others => '1');
    report "logic others: " & integer'image(n);
    n := 0; r.f(t(1) to 2) <= (5, 6);
    report "record field: " & integer'image(n);
    n := 0; a(1)(t(1) to 2) <= (5, 6);
    report "element of an array of arrays: " & integer'image(n);
    n := 0; y(t(1) to 2) <= (others => (4, 5));
    report "array of arrays: " & integer'image(n);
    n := 0; x(t(1) to 2) <= x(0 to 1);
    report "array of records, plain value: " & integer'image(n);
    n := 0; x(t(1) to 2) <= (others => (3, (4, 5)));
    report "array of records: " & integer'image(n);
    n := 0; x(t(1) to 2) <= (others => (3, (4, 5))), (others => (6, (7, 8))) after 1 ns;
    report "array of records, two elements: " & integer'image(n);
    n := 0; al(t(1) to 2) <= (5, 6);
    report "alias: " & integer'image(n);
    n := 0; p(s4);
    report "signal parameter: " & integer'image(n);
    n := 0; sf(t(1) to 2) <= force (5, 6);
    report "force: " & integer'image(n);
    n := 0; xf(t(1) to 2) <= force (others => (3, (4, 5)));
    report "force, array of records: " & integer'image(n);
    wait for 5 ns;
    report "values " & integer'image(s(1)) & integer'image(s(2)) & integer'image(s2(1))
      & integer'image(s3(2)) & integer'image(r.f(2)) & integer'image(a(1)(2))
      & integer'image(s4(2)) & integer'image(sf(2)) & integer'image(sa(2))
      & integer'image(x(2).b(1)) & integer'image(xf(2).a) & integer'image(y(2)(1))
      & integer'image(gs(2)) & " " & std_ulogic'image(l(0)) & std_ulogic'image(l(2));
    n := 0; xf(t(1) to 2) <= release;
    report "release: " & integer'image(n);
    wait;
  end process;
end architecture;
