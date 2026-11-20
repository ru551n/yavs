-- difftest: top=formal_range_once std=2008 ref=hand
-- ref=hand: sim1 calls the function in the range of an out port at an assignment to the
-- port, not every one during elaboration, so its first count is 15 (discrepancy
-- `out-port-range-called-again`); sim2 agrees with this output (it needs the actuals' ranges
-- to match the formals').
-- The subtype of a formal generic or port is elaborated once, with the generic or port
-- clause of its entity, component or block (LRM 14.3.2, 14.4.2.1): an impure function in a
-- range is called once per formal and instance, not again by the checks of the generic's
-- value, the port's association or an assignment to the port (#1177).
package formal_range_once_c is
  type counter is protected
    impure function bump (by : integer) return integer;
  end protected;
end package;
package body formal_range_once_c is
  type counter is protected body
    variable v : integer := 0;
    impure function bump (by : integer) return integer is
    begin
      v := v + by;
      return v;
    end function;
  end protected body;
end package body;
use work.formal_range_once_c.all;
package formal_range_once_p is
  shared variable c : counter;
  impure function counted (x : integer) return integer;
  impure function calls return integer;
end package;
package body formal_range_once_p is
  impure function counted (x : integer) return integer is
    variable n : integer;
  begin
    n := c.bump(1);
    return x;
  end function;
  impure function calls return integer is begin return c.bump(0); end function;
end package body;
use work.formal_range_once_p.all;
entity formal_range_once_sub is
  generic (g : integer range 0 to counted(30) := 1);
  port (p : in integer range 0 to counted(20) := 0; o : out integer range 0 to counted(21) := 0);
end entity;
architecture a of formal_range_once_sub is
begin
  process begin
    wait for 1 ns;
    o <= p + g;
    wait for 1 ns;
    o <= p + g + 1;
    wait;
  end process;
end architecture;
architecture b of formal_range_once_sub is
begin
  process begin
    wait for 1 ns;
    o <= p + 2 * g;
    wait;
  end process;
end architecture;
use work.formal_range_once_p.all;
-- Array formals: the index ranges of their anonymous subtypes.
entity formal_range_once_arr is
  generic (g : bit_vector(0 to counted(3)) := "0000");
  port (p : in bit_vector(0 to counted(7)); o : out bit_vector(0 to counted(7)));
end entity;
architecture a of formal_range_once_arr is
begin
  process begin
    wait for 1 ns;
    o <= p xor (p'range => g(0));
    wait;
  end process;
end architecture;
use work.formal_range_once_p.all;
entity formal_range_once is
  generic (t : integer range 0 to counted(40) := 3);
end entity;
architecture a of formal_range_once is
  component formal_range_once_sub is
    generic (g : integer range 0 to counted(30) := 1);
    port (p : in integer range 0 to counted(20) := 0; o : out integer range 0 to counted(21) := 0);
  end component;
  signal a : integer range 0 to 20 := 0;
  signal b, c2, d : integer range 0 to 21 := 0;
  signal e : integer range 0 to 61 := 0;
  signal va, vb : bit_vector(0 to 7) := (others => '0');
begin
  u1 : entity work.formal_range_once_sub generic map (g => 2) port map (p => a, o => b);
  -- Defaults: an open port and a generic without an actual.
  u2 : entity work.formal_range_once_sub(b) port map (p => open, o => c2);
  -- The component's formals and the entity's.
  u3 : component formal_range_once_sub generic map (g => 4) port map (p => a, o => d);
  u4 : entity work.formal_range_once_arr generic map (g => "1010") port map (p => va, o => vb);
  blk : block
    generic (bg : integer range 0 to counted(50));
    generic map (bg => 5);
    port (bp : in integer range 0 to counted(20); bo : out integer range 0 to counted(61));
    port map (bp => a, bo => e);
  begin
    process begin
      wait for 1 ns;
      bo <= bp + bg;
      wait;
    end process;
  end block;
  process begin
    report "start calls=" & integer'image(calls);
    a <= 5;
    va <= "11110000";
    wait for 3 ns;
    report "b=" & integer'image(b) & " c2=" & integer'image(c2) & " d=" & integer'image(d)
      & " e=" & integer'image(e) & " t=" & integer'image(t) & " vb=" & to_string(vb);
    report "end calls=" & integer'image(calls);
    wait;
  end process;
end architecture;
