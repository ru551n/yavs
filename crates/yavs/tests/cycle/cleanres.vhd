-- --two-state: resolved signals (std_logic_vector, numeric_std's unsigned) with one driver that
-- drives only 0 and 1 are read without a map; one with two drivers, a driver that can drive 'Z'
-- (a literal, an aggregate, a constant, through an alias of an unresolved signal), forced, fed
-- through a port by a source that does not qualify, or driven by four-state code (a package's
-- procedure through a signal out or inout parameter, in a process or a concurrent call; a record
-- or slice variable it wrote) is still mapped ('1' and 'H' are 1, anything else 0). `ones`
-- counts the elements read as 1, and those read as 0 in hundreds.
library ieee;
use ieee.std_logic_1164.all;

package cleanres_pkg is
  type rec_t is record
    a : std_ulogic_vector(3 downto 0);
  end record;
  procedure put_x(signal s : out std_logic_vector(3 downto 0));
  procedure put_xu(signal s : out std_ulogic_vector(3 downto 0));
  procedure put_io(signal s : inout std_ulogic_vector(3 downto 0));
  procedure put_rec(variable r : out rec_t);
  procedure put_v(variable v : out std_ulogic_vector(3 downto 0));
end package;

package body cleanres_pkg is
  procedure put_x(signal s : out std_logic_vector(3 downto 0)) is
  begin
    s <= "X1U1";
  end procedure;

  procedure put_xu(signal s : out std_ulogic_vector(3 downto 0)) is
  begin
    s <= "X1U1";
  end procedure;

  procedure put_io(signal s : inout std_ulogic_vector(3 downto 0)) is
  begin
    s <= "X1U1";
  end procedure;

  procedure put_rec(variable r : out rec_t) is
  begin
    r.a := "X1U1";
  end procedure;

  procedure put_v(variable v : out std_ulogic_vector(3 downto 0)) is
  begin
    v := "X1U1";
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;

entity cleanres_sub is
  port (clk : in std_ulogic; o : out std_logic_vector(3 downto 0) := "U1X1");
end entity;

architecture rtl of cleanres_sub is
begin
  process (clk)
  begin
    if falling_edge(clk) then
      o <= "1111";
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cleanres_pkg.all;

entity cleanres_dut is
  port (
    clk : in std_ulogic;
    pin : in std_logic_vector(3 downto 0);
    n : out integer_vector(0 to 15));
end entity;

architecture rtl of cleanres_dut is
  constant CZ : std_logic_vector(3 downto 0) := "1Z0Z";
  signal r_ok : std_logic_vector(3 downto 0);
  signal u_ok : unsigned(3 downto 0);
  signal r_two, r_z, r_agg, r_c, r_pkg, r_pout : std_logic_vector(3 downto 0);
  signal r_forced : std_logic_vector(3 downto 0) := "0000";
  signal u_pkg, u_io, u_cc, u_rec, u_al : std_ulogic_vector(3 downto 0);
  signal u_var : std_ulogic_vector(7 downto 0);
  alias a_al : std_logic_vector(3 downto 0) is u_al;
  function ones(v : std_ulogic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        c := c + 1;
      elsif v(i) = '0' then
        c := c + 100;
      end if;
    end loop;
    return c;
  end function;
begin
  sub : entity work.cleanres_sub port map (clk, r_pout);
  r_two <= "0011";
  r_two <= "0101";
  r_z <= "Z1Z1";
  r_agg <= (3 => 'Z', others => '1');
  r_c <= CZ;
  put_xu(u_cc);
  process
    variable r : rec_t;
    variable w : std_ulogic_vector(7 downto 0) := (others => '0');
  begin
    put_x(r_pkg);
    put_xu(u_pkg);
    put_io(u_io);
    put_rec(r);
    u_rec <= r.a;
    put_v(w(3 downto 0));
    u_var <= w;
    a_al <= "Z1Z1";
    wait;
  end process;
  process (clk)
    variable c : integer_vector(0 to 15) := (others => 0);
  begin
    if rising_edge(clk) then
      r_ok <= not r_ok;
      u_ok <= u_ok + 5;
      c := (c(0) + ones(std_ulogic_vector(r_ok)), c(1) + ones(std_ulogic_vector(u_ok)),
        c(2) + ones(std_ulogic_vector(r_two)), c(3) + ones(std_ulogic_vector(r_z)),
        c(4) + ones(std_ulogic_vector(r_agg)), c(5) + ones(std_ulogic_vector(r_c)),
        c(6) + ones(std_ulogic_vector(r_forced)), c(7) + ones(std_ulogic_vector(r_pkg)),
        c(8) + ones(u_pkg), c(9) + ones(std_ulogic_vector(r_pout)),
        c(10) + ones(std_ulogic_vector(pin)), c(11) + ones(u_io), c(12) + ones(u_cc),
        c(13) + ones(u_rec), c(14) + ones(u_var), c(15) + ones(u_al));
      n <= c;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cleanres is
end entity;

architecture tb of cleanres is
  signal clk : std_ulogic := '0';
  signal n : integer_vector(0 to 15);
  signal x : std_logic_vector(3 downto 0) := "X1U1";
  signal y : std_logic_vector(3 downto 0);
begin
  -- A resolved port the testbench feeds through a concurrent copy.
  y <= x;
  dut : entity work.cleanres_dut port map (clk, y, n);
  process
  begin
    << signal .cleanres.dut.r_forced : std_logic_vector(3 downto 0) >> <= force "1UX1";
    for i in 1 to 4 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "ok=" & integer'image(n(0)) & " unsigned=" & integer'image(n(1)) & " two="
      & integer'image(n(2)) & " z=" & integer'image(n(3)) & " agg=" & integer'image(n(4))
      & " const=" & integer'image(n(5)) & " forced=" & integer'image(n(6)) & " pkg="
      & integer'image(n(7)) & " upkg=" & integer'image(n(8)) & " port=" & integer'image(n(9))
      & " pin=" & integer'image(n(10)) & " io=" & integer'image(n(11)) & " cc="
      & integer'image(n(12)) & " rec=" & integer'image(n(13)) & " var=" & integer'image(n(14))
      & " alias=" & integer'image(n(15));
    wait;
  end process;
end architecture;
