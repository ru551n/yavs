-- difftest: top=port_conv_record std=2008
-- Conversion functions of record ports in a port map (#74): one kernel converter over all
-- leaves of a record or an array of records, run in the update phase without a delta,
-- into and out of the instance.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package port_conv_record_pkg is
  type pair is record
    hi : std_logic_vector(3 downto 0);
    lo : integer range 0 to 15;
  end record;
  type wide is record
    n : natural;
    ok : boolean;
  end record;
  type pairs is array (natural range <>) of pair;
  function to_wide(p : pair) return wide;
  function to_ints(ps : pairs) return integer_vector;
  function to_pair(w : wide) return pair;
  function to_int(p : pair) return integer;
  function to_pair(i : integer) return pair;
end package;
package body port_conv_record_pkg is
  function to_wide(p : pair) return wide is
  begin
    return (n => to_integer(unsigned(p.hi)) * 16 + p.lo, ok => p.hi /= "0000");
  end function;
  function to_pair(w : wide) return pair is
  begin
    return (hi => std_logic_vector(to_unsigned(w.n / 16 mod 16, 4)), lo => w.n mod 16);
  end function;
  function to_ints(ps : pairs) return integer_vector is
    variable r : integer_vector(ps'range);
  begin
    for k in ps'range loop
      r(k) := to_int(ps(k));
    end loop;
    return r;
  end function;
  function to_int(p : pair) return integer is
  begin
    return to_integer(unsigned(p.hi)) * 16 + p.lo;
  end function;
  function to_pair(i : integer) return pair is
  begin
    return (hi => std_logic_vector(to_unsigned(i / 16 mod 16, 4)), lo => i mod 16);
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.port_conv_record_pkg.all;
entity port_conv_record_sub is
  port (
    w : in wide;
    v : out wide;
    i : in integer;
    o : out integer;
    x : in integer_vector(0 to 1)
  );
end entity;
architecture a of port_conv_record_sub is
begin
  v <= (n => w.n + 1, ok => not w.ok);
  o <= i * 2;
  process (w) begin
    report "sub w " & integer'image(w.n) & " " & boolean'image(w.ok);
  end process;
  process (x) begin
    report "sub x " & integer'image(x(0)) & " " & integer'image(x(1));
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.port_conv_record_pkg.all;
entity port_conv_record is end;
architecture t of port_conv_record is
  signal p : pair := ("0001", 2);
  signal q : pair;
  signal r : pair := ("0000", 3);
  signal s : pair;
  signal ps : pairs(0 to 1) := (("0001", 1), ("0010", 2));
begin
  u : entity work.port_conv_record_sub port map (
    w => to_wide(p),
    to_pair(v) => q,
    i => to_int(r),
    to_pair(o) => s,
    x => to_ints(ps));

  process
  begin
    wait for 1 ns;
    report "q " & to_string(q.hi) & " " & integer'image(q.lo) & " s " & to_string(s.hi)
      & " " & integer'image(s.lo);
    assert q.hi = "0001" and q.lo = 3 and s.hi = "0000" and s.lo = 6;
    p <= ("0010", 15);
    r <= ("0001", 1);
    wait for 0 ns;
    -- `w` follows `p` in this delta, `q` follows `v` one delta later.
    assert q.hi = "0001" and q.lo = 3;
    wait for 0 ns;
    report "delta2 q " & to_string(q.hi) & " " & integer'image(q.lo) & " s "
      & to_string(s.hi) & " " & integer'image(s.lo);
    assert q.hi = "0011" and q.lo = 0 and s.hi = "0010" and s.lo = 2;
    p.lo <= 4;
    wait for 1 ns;
    report "q " & to_string(q.hi) & " " & integer'image(q.lo);
    assert q.hi = "0010" and q.lo = 5;
    ps(1).hi <= "0100";
    wait;
  end process;
end architecture;
