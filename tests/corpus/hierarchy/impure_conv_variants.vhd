-- difftest: top=tb_impure_conv_variants std=2008 ignore=order
-- Impure conversion functions in port maps that read signals of the instantiating region
-- (#1166): formal side of an `out` port, both sides of an `inout` port, a record port, a
-- record signal read, several signals read, an entity port read, a generate iteration's
-- own signal. A change of a signal read alone converts nothing (LRM 6.5.7.1). ignore=order:
-- reports of different processes in one cycle.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package pk_icv is
  type rec_t is record
    a : unsigned(3 downto 0);
    b : std_logic;
    n : integer;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity src_icv is
  port (o : out unsigned(7 downto 0));
end entity;
architecture a of src_icv is
begin
  process
  begin
    o <= x"12";
    wait for 1 ns;
    o <= x"34";
    wait for 3 ns;
    o <= x"56";
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity bidir_icv is
  port (io : inout std_logic_vector(7 downto 0));
end entity;
architecture a of bidir_icv is
begin
  process
  begin
    io <= x"21";
    wait for 2 ns;
    io <= x"43";
    wait;
  end process;
  process (io)
  begin
    report "io=" & to_hstring(io);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.pk_icv.all;
entity rin_icv is
  port (r : in rec_t; v : in unsigned(7 downto 0));
end entity;
architecture a of rin_icv is
begin
  process (r, v)
  begin
    report "r=" & to_hstring(r.a) & " " & std_logic'image(r.b) & " " & integer'image(r.n) &
      " v=" & to_hstring(v);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity sink_icv is
  port (b : in unsigned(7 downto 0));
end entity;
architecture a of sink_icv is
begin
  process (b)
  begin
    report sink_icv'instance_name & " b=" & to_hstring(b);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity wrap_icv is
  port (p : in unsigned(7 downto 0));
end entity;
architecture a of wrap_icv is
  signal q : unsigned(7 downto 0) := x"05";
  impure function add_p(x : unsigned(7 downto 0)) return unsigned is
  begin
    return x + p;
  end function;
begin
  u : entity work.sink_icv port map (b => add_p(q));
  process
  begin
    wait for 5 ns;
    q <= x"06";
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.pk_icv.all;
entity tb_impure_conv_variants is
end entity;
architecture a of tb_impure_conv_variants is
  signal y : unsigned(7 downto 0) := x"01";
  signal k : integer := 7;
  signal sl : std_logic := '1';
  signal rs : rec_t := (a => x"3", b => '0', n => 100);
  signal z, x : unsigned(7 downto 0) := x"00";
  signal w : std_logic_vector(7 downto 0);
  impure function f(p : unsigned(7 downto 0)) return unsigned is
  begin
    return p xor y;
  end function;
  impure function to_bus(p : std_logic_vector(7 downto 0)) return std_logic_vector is
  begin
    return p xor std_logic_vector(y);
  end function;
  impure function from_bus(p : std_logic_vector(7 downto 0)) return std_logic_vector is
  begin
    return p xor std_logic_vector(rs.a & rs.a);
  end function;
  impure function to_rec(p : unsigned(7 downto 0)) return rec_t is
  begin
    return (a => p(3 downto 0) + rs.a, b => sl, n => k + rs.n);
  end function;
  impure function mix(p : unsigned(7 downto 0)) return unsigned is
  begin
    return p + to_unsigned(k, 8) + y;
  end function;
begin
  o1 : entity work.src_icv port map (f(o) => z);
  b1 : entity work.bidir_icv port map (to_bus(io) => from_bus(w));
  r1 : entity work.rin_icv port map (r => to_rec(x), v => mix(x));
  w1 : entity work.wrap_icv port map (p => y);
  g : for i in 1 to 2 generate
    signal s : unsigned(7 downto 0) := to_unsigned(16 * i, 8);
    impure function h(p : unsigned(7 downto 0)) return unsigned is
    begin
      return p + s;
    end function;
  begin
    u : entity work.sink_icv port map (b => h(x));
    process
    begin
      wait for 6 ns;
      s <= s + 1;
      wait for 1 ns;
      s <= s + 1;
      wait;
    end process;
  end generate;
  process (z, w)
  begin
    report "z=" & to_hstring(z) & " w=" & to_hstring(w);
  end process;
  process
  begin
    wait for 2500 ps;
    y <= x"10";
    k <= 8;
    sl <= '0';
    rs.a <= x"1";
    wait for 1 ns;
    x <= x"0A";
    wait for 4 ns;
    x <= x"0B";
    wait;
  end process;
end architecture;
