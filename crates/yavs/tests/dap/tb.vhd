entity tb is
  generic (N : integer := 3; NAME : string := "dap");
end entity;

architecture a of tb is
  signal cnt : integer := 0;
  signal v : bit_vector(3 downto 0) := "0101";

  function twice(x : integer) return integer is
    variable r : integer;
  begin
    r := x * 2;
    return r;
  end function;
begin
  p : process
    variable i : integer := 0;
  begin
    for k in 1 to N loop
      i := twice(k);
      cnt <= i;
      wait for 10 ns;
    end loop;
    report "done";
    wait;
  end process;

  q : process (cnt)
  begin
    v <= not v;
  end process;
end architecture;

-- Runs until it is stopped: for the pause test.
entity spin is
end entity;

architecture a of spin is
  signal cnt : natural := 0;
begin
  process
  begin
    loop
      cnt <= cnt + 1;
      wait for 1 ns;
    end loop;
  end process;
end architecture;

-- Composite signals and variables: deposit, force, release, setVariable, composite views.
library ieee;
use ieee.std_logic_1164.all;

entity vals is
end entity;

architecture a of vals is
  type rec_t is record
    a : integer;
    b : std_logic_vector(3 downto 0);
  end record;
  type arr_t is array (0 to 299) of natural;
  signal r : rec_t := (a => 1, b => "0000");
  signal d : std_logic_vector(7 downto 0) := x"00";
  signal t : time := 0 ns;
  signal q : std_logic_vector(7 downto 0);
begin
  q <= d;

  p : process
    variable v : rec_t := (a => 5, b => "1010");
    variable big : arr_t := (others => 7);
    variable n : integer := 0;
  begin
    wait for 10 ns;
    n := v.a + 1;
    wait for 1 ns;
    report "n=" & integer'image(n) & " b=" & to_hstring(v.b) & " d=" & to_hstring(d)
      & " q=" & to_hstring(q) & " t=" & time'image(t) & " r.a=" & integer'image(r.a);
    wait for 1 ns;
    report "q=" & to_hstring(q) & " big(299)=" & integer'image(big(299));
    wait;
  end process;
end architecture;

-- A timed wait woken by events that leave its condition false, while the session runs, pauses
-- in it and steps over it: the timeout still ends it at the time of its statement.
entity tw is
end entity;

architecture a of tw is
  signal s : integer := 0;
begin
  stim : process
  begin
    for k in 1 to 3 loop
      wait for 10 ns;
      s <= k;
    end loop;
    wait;
  end process;

  w : process
  begin
    wait on s until s = 99 for 45 ns;
    report "first at " & time'image(now) & " s=" & integer'image(s);
    wait on s until s = 99 for 10 ns;
    report "second at " & time'image(now);
    wait;
  end process;
end architecture;

-- Conditions that call functions, convert types, use the operators of numeric_std and
-- std_logic_1164 and read attributes (#364).
package calls_pkg is
  type int_arr is array (0 to 3) of integer;
  function make return int_arr;
  constant TABLE : int_arr;
  function lookup(i : natural) return integer;
  -- Impure functions: one that only reads, and ones that write.
  impure function since(t : time) return time;
  type counter_t is protected
    procedure inc;
  end protected;
  impure function quit(x : integer) return integer;
  impure function logged(x : integer) return integer;
end package;

package body calls_pkg is
  function make return int_arr is
  begin
    return (10, 20, 30, 40);
  end function;

  constant TABLE : int_arr := make;

  function lookup(i : natural) return integer is
  begin
    return TABLE(i);
  end function;

  impure function since(t : time) return time is
  begin
    return now - t;
  end function;

  type counter_t is protected body
    variable n : natural := 0;
    procedure inc is
    begin
      n := n + 1;
    end procedure;
  end protected body;

  impure function quit(x : integer) return integer is
  begin
    std.env.stop;
    return x;
  end function;

  impure function logged(x : integer) return integer is
    variable l : std.textio.line;
  begin
    std.textio.write(l, x);
    std.textio.writeline(std.textio.output, l);
    return x;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.calls_pkg.all;

entity calls is
  generic (W : natural := 4);
end entity;

architecture a of calls is
  signal clk : std_logic := '0';
  signal en : std_logic := '1';
  signal d : std_logic_vector(7 downto 0) := x"00";
  signal u : unsigned(3 downto 0) := "0000";
  signal seen : natural := 0;
  constant K : integer := 3;
  constant K2 : integer := W + 1;
  type rec_t is record
    b : std_logic_vector(3 downto 0);
    n : unsigned(3 downto 0);
  end record;
  signal r : rec_t := ("0101", "0011");

  function plus_k(x : integer) return integer is
  begin
    return x + K;
  end function;

  function times_w(x : integer) return integer is
  begin
    return x * W;
  end function;

  function plus_k2(x : integer) return integer is
  begin
    return x + K2;
  end function;

  procedure check(x : unsigned) is
    variable y : unsigned(3 downto 0);
  begin
    y := x;
    assert y /= 15;
  end procedure;

  function noisy(x : integer) return integer is
  begin
    report "noisy called" severity error;
    return x;
  end function;

  impure function bump(x : integer) return natural is
  begin
    return x + seen;
  end function;

  impure function edges return natural is
  begin
    return seen;
  end function;

  shared variable hits : counter_t;

  impure function tick(x : integer) return integer is
  begin
    hits.inc;
    return x;
  end function;

  impure function via_tick(x : integer) return integer is
  begin
    return tick(x) + 1;
  end function;

  impure function rose(x : integer) return boolean is
  begin
    return clk'event;
  end function;
begin
  p : process
    variable v : unsigned(3 downto 0) := "0000";
  begin
    for k in 1 to 6 loop
      clk <= not clk;
      d <= std_logic_vector(to_unsigned(k * 16 + k, 8));
      u <= u + 1;
      v := v + 2;
      check(u);
      wait for 5 ns;
    end loop;
    report "u=" & to_hstring(u) & " v=" & to_hstring(v) & " seen=" & integer'image(seen);
    wait;
  end process;

  q : process (clk)
    function twice(x : integer) return integer is
    begin
      return 2 * x;
    end function;
  begin
    seen <= seen + 1;
  end process;
end architecture;

-- The same function of two instances reads the signal of the instance it is called from (#364).
entity calls_sub is
  generic (N : natural);
end entity;

architecture a of calls_sub is
  signal s : natural := N * 10;

  impure function get_s(x : integer) return integer is
  begin
    return s + x;
  end function;
begin
  p : process
  begin
    wait for 1 ns;
    s <= s + 1;
    wait;
  end process;
end architecture;

entity calls_two is
end entity;

architecture a of calls_two is
begin
  u1 : entity work.calls_sub generic map (N => 1);
  u2 : entity work.calls_sub generic map (N => 2);
end architecture;
