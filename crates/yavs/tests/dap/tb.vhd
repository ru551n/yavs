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
