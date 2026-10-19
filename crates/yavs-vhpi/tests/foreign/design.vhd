-- Foreign subprograms implemented by the VHPI plugin tests/plugin/foreign.c.
library ieee;
use ieee.std_logic_1164.all;

package models is
  function add(a, b : integer) return integer;
  attribute foreign of add : function is "VHPI plug add";
  function half(x : real) return real;
  attribute foreign of half : function is "VHPI plug half";
  function later(t : time) return time;
  attribute foreign of later : function is "VHPI plug later";
  function parity(v : std_logic_vector) return std_logic;
  attribute foreign of parity : function is "VHPI plug parity";
  function is_big(x : integer) return boolean;
  attribute foreign of is_big : function is "VHPI plug is_big";
  procedure divmod(a, b : in integer; q, r : out integer);
  attribute foreign of divmod : procedure is "VHPI plug divmod";
  procedure invert(v : inout std_logic_vector);
  attribute foreign of invert : procedure is "VHPI plug invert";
  procedure bump(x : inout integer);
  attribute foreign of bump : procedure is "VHPI plug bump";
  function forgetful(x : integer) return integer;
  attribute foreign of forgetful : function is "VHPI plug forgetful";
  function missing(x : integer) return integer;
  attribute foreign of missing : function is "VHPI plug missing";
end package;

package body models is
  -- Placeholder bodies: the foreign attribute wins.
  function add(a, b : integer) return integer is begin return 0; end;
  function half(x : real) return real is begin return 0.0; end;
  function later(t : time) return time is begin return t; end;
  function parity(v : std_logic_vector) return std_logic is begin return 'U'; end;
  function is_big(x : integer) return boolean is begin return false; end;
  procedure divmod(a, b : in integer; q, r : out integer) is begin q := 0; r := 0; end;
  procedure invert(v : inout std_logic_vector) is begin end;
  procedure bump(x : inout integer) is begin end;
  function forgetful(x : integer) return integer is begin return 0; end;
  function missing(x : integer) return integer is begin return 0; end;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.models.all;

entity top is end;
architecture a of top is begin
  process
    variable q, r, n : integer;
    variable v : std_logic_vector(3 downto 0) := "1100";
  begin
    assert add(40, 2) = 42 report "add" severity failure;
    assert add(-5, 3) = -2 report "add negative" severity failure;
    assert half(5.0) = 2.5 report "half" severity failure;
    assert later(3 ns) = 4 ns report "later" severity failure;
    assert parity("1011") = '1' report "parity odd" severity failure;
    assert parity("1001") = '0' report "parity even" severity failure;
    assert is_big(100) and not is_big(3) report "is_big" severity failure;
    divmod(17, 5, q, r);
    assert q = 3 and r = 2 report "divmod" severity failure;
    invert(v);
    assert v = "0011" report "invert" severity failure;
    n := 6;
    bump(n);
    assert n = 7 report "bump" severity failure;
    report "foreign ok";
    wait;
  end process;
end architecture;

use work.models.all;
entity nores is end;
architecture a of nores is begin
  process begin report integer'image(forgetful(1)); wait; end process;
end architecture;

use work.models.all;
entity lazy is end;
architecture a of lazy is begin
  process
    variable n : integer := 0;
  begin
    if n > 0 then report integer'image(missing(1)); end if;
    report "lazy ok";
    wait;
  end process;
end architecture;

use work.models.all;
entity miss is end;
architecture a of miss is begin
  process begin report integer'image(missing(1)); wait; end process;
end architecture;
