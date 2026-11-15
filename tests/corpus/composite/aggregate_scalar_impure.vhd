-- difftest: top=aggregate_scalar_impure std=2008 ref=hand
-- An impure call in a choice of a scalar array aggregate that names several elements runs
-- once per element, `others` after the other choices (LRM 9.3.3, #909, #1158). ref=hand: the
-- values are sim2's; sim1 runs the expression of an `others` or range choice once.
library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;
entity aggregate_scalar_impure is end;
architecture a of aggregate_scalar_impure is
  type iv is array (0 to 3) of integer;
  type dv is array (3 downto 0) of integer;
  type rv is array (0 to 2) of real;
  type ev is array (character range 'a' to 'c') of integer;
  type im2 is array (0 to 1, 0 to 1) of integer;
  type counter_t is protected
    impure function next_id return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is begin n := n + 1; return n; end function;
  end protected body;
  shared variable c : counter_t;
  signal sg : iv;
  function im(x : integer_vector) return string is
    variable l : line;
  begin
    for i in x'range loop
      write(l, integer'image(x(i)) & " ");
    end loop;
    return l.all;
  end function;
  function imv(x : iv) return string is begin return im(integer_vector(x)); end;
  function ime(x : ev) return string is begin return im((x('a'), x('b'), x('c'))); end;
begin
  process
    variable n : natural := 0;
    impure function t return natural is begin n := n + 1; return n; end function;
    impure function b return std_ulogic is begin n := n + 1; if n mod 2 = 0 then return '1'; else return '0'; end if; end function;
    impure function r return real is begin n := n + 1; return real(n); end function;
    impure function mk return iv is begin return (1 to 2 => t, others => t); end function;
    variable vi : iv := (others => t);
    variable v : iv;
    variable d : dv;
    variable s : std_ulogic_vector(0 to 5);
    variable w : rv;
    variable e : ev;
    variable m : im2;
    variable k : integer := 1;
  begin
    report "initial: " & imv(vi) & "calls " & integer'image(n);
    n := 0; v := (others => t); report "others: " & imv(v) & "calls " & integer'image(n);
    n := 0; v := (0 to 1 => t, others => t); report "range+others: " & imv(v) & "calls " & integer'image(n);
    n := 0; v := (2 => 77, others => t); report "elem+others: " & imv(v) & "calls " & integer'image(n);
    n := 0; v := (0 to 3 => t); report "range: " & imv(v) & "calls " & integer'image(n);
    n := 0; v := (1 | 3 => t, others => 9); report "list: " & imv(v) & "calls " & integer'image(n);
    n := 0; v(0 to 2) := (k to 3 => t); report "dyn range: " & imv(v) & "calls " & integer'image(n);
    n := 0; v := (t, t, others => t); report "pos+others: " & imv(v) & "calls " & integer'image(n);
    n := 0; v := (others => t + 10); report "nested: " & imv(v) & "calls " & integer'image(n);
    n := 0; v := (others => 5); report "pure others: " & imv(v);
    v := (0 to 1 => c.next_id, others => c.next_id); report "method: " & imv(v);
    n := 0; d := (3 downto 2 => t, others => t); report "downto: " & im(integer_vector(d)) & "calls " & integer'image(n);
    n := 0; e := ('b' => 50, others => t); report "enum index: " & ime(e) & "calls " & integer'image(n);
    n := 0; report "named bounds: " & im(integer_vector'(2 to 4 => t)) & "calls " & integer'image(n);
    n := 0; report "actual: " & imv((others => t)) & "calls " & integer'image(n);
    n := 0; v := mk; report "return: " & imv(v) & "calls " & integer'image(n);
    n := 0; sg <= (1 to 2 => t, others => t); wait for 1 ns; report "signal: " & imv(sg) & "calls " & integer'image(n);
    n := 0; s := (others => b); report "logic others: " & to_string(s) & " calls " & integer'image(n);
    n := 0; s := (1 to 2 => b, others => '1'); report "logic range: " & to_string(s) & " calls " & integer'image(n);
    n := 0; s := (s(0 to 1), others => b); report "logic pieces: " & to_string(s) & " calls " & integer'image(n);
    n := 0; s := (4 to 5 => s(0 to 1), others => b); report "named piece: " & to_string(s) & " calls " & integer'image(n);
    n := 0; s := (0 to 3 => s(2 to 5), 4 to 5 => b); report "piece range: " & to_string(s) & " calls " & integer'image(n);
    n := 0; w := (others => r); report "real others: " & integer'image(integer(w(0))) & " " & integer'image(integer(w(1))) & " " & integer'image(integer(w(2))) & " calls " & integer'image(n);
    n := 0; m := (others => (others => t));
    report "2d others: " & im(integer_vector'(m(0, 0), m(0, 1), m(1, 0), m(1, 1))) & "calls " & integer'image(n);
    wait;
  end process;
end;
