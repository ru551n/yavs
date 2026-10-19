-- difftest: top=port_conv_inout_formal_only_composite std=2008 ref=hand
-- LRM 6.5.7.1: an inout record port and an inout array-of-records port with a conversion on
-- the formal side only (inc(q) => s, #74). Copy-out converts every leaf in one call, copy-in
-- passes the actual's leaves unchanged, so a second driver of the actual reaches the formal.
-- ref=hand: the first reference simulator leaves the formal at its own driven value; the
-- third gives these results.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package port_conv_inout_formal_only_composite_pkg is
  type pair is record
    a : std_logic_vector(3 downto 0);
    b : std_logic_vector(3 downto 0);
  end record;
  type pairs is array (0 to 1) of pair;
  function inc(p : pair) return pair;
  function inc(ps : pairs) return pairs;
end package;
package body port_conv_inout_formal_only_composite_pkg is
  function inc(p : pair) return pair is
  begin
    return (a => std_logic_vector(unsigned(p.a) + 1), b => p.b);
  end function;
  function inc(ps : pairs) return pairs is
  begin
    return (inc(ps(0)), inc(ps(1)));
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.port_conv_inout_formal_only_composite_pkg.all;
entity port_conv_inout_formal_only_composite_sub is
  port (
    q : inout pair := ("0000", "0000");
    qs : inout pairs := (("0000", "0000"), ("0000", "0000"))
  );
end entity;
architecture a of port_conv_inout_formal_only_composite_sub is
begin
  process begin
    wait for 1 ns;
    q <= ("0011", "0101");
    qs <= (("0001", "0010"), ("0111", "0000"));
    wait for 1 ns;
    report "sub q " & to_string(q.a) & " " & to_string(q.b);
    report "sub qs " & to_string(qs(0).a) & " " & to_string(qs(0).b) & " "
      & to_string(qs(1).a) & " " & to_string(qs(1).b);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.port_conv_inout_formal_only_composite_pkg.all;
entity port_conv_inout_formal_only_composite is end entity;
architecture t of port_conv_inout_formal_only_composite is
  signal s : pair;
  signal ss : pairs;
begin
  u : entity work.port_conv_inout_formal_only_composite_sub
    port map (inc(q) => s, inc(qs) => ss);

  -- A second driver of the actuals: a `1` against the port's `0` makes bit 3 of `b` X.
  s.b <= "ZZZZ", "1ZZZ" after 1500 ps;
  ss(1).b <= "ZZZZ", "1ZZZ" after 1500 ps;

  process begin
    wait for 3 ns;
    report "s " & to_string(s.a) & " " & to_string(s.b);
    report "ss " & to_string(ss(0).a) & " " & to_string(ss(0).b) & " "
      & to_string(ss(1).a) & " " & to_string(ss(1).b);
    assert s.a = "0100" and s.b = "X101";
    assert ss(0).a = "0010" and ss(0).b = "0010" and ss(1).a = "1000" and ss(1).b = "X000";
    wait;
  end process;
end architecture;
