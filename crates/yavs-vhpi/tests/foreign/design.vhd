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
  -- Array results (#330): sized by the model, or by a constrained result subtype.
  function rev(v : std_logic_vector) return std_logic_vector;
  attribute foreign of rev : function is "VHPI plug rev";
  function iota(n : integer) return integer_vector;
  attribute foreign of iota : function is "VHPI plug iota";
  function word(n : integer) return string;
  attribute foreign of word : function is "VHPI plug word";
  subtype nib is std_logic_vector(3 downto 0);
  function nibble(x : integer) return nib;
  attribute foreign of nibble : function is "VHPI plug nibble";
  function unsized(x : integer) return integer_vector;
  attribute foreign of unsized : function is "VHPI plug unsized";
  -- Record, access and signal parameters (#330).
  type pt is record
    x   : integer;
    ok  : boolean;
    tag : std_logic_vector(3 downto 0);
  end record;
  function norm(p : pt) return integer;
  attribute foreign of norm : function is "VHPI plug norm";
  procedure flip(p : in pt; q : out pt);
  attribute foreign of flip : procedure is "VHPI plug flip";
  type int_ptr is access integer;
  procedure peek(variable p : in int_ptr; r : out integer);
  attribute foreign of peek : procedure is "VHPI plug peek";
  procedure drive(signal s : in std_logic_vector; signal o : out std_logic_vector;
                  signal k : out integer);
  attribute foreign of drive : procedure is "VHPI plug drive";
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
  function rev(v : std_logic_vector) return std_logic_vector is begin return v; end;
  function iota(n : integer) return integer_vector is begin return (1 to 0 => 0); end;
  function word(n : integer) return string is begin return ""; end;
  function nibble(x : integer) return nib is begin return "UUUU"; end;
  function unsized(x : integer) return integer_vector is begin return (1 to 0 => 0); end;
  function norm(p : pt) return integer is begin return 0; end;
  procedure flip(p : in pt; q : out pt) is begin end;
  procedure peek(variable p : in int_ptr; r : out integer) is begin end;
  procedure drive(signal s : in std_logic_vector; signal o : out std_logic_vector;
                  signal k : out integer) is begin end;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.models.all;

entity top is end;
architecture a of top is
  signal s, o : std_logic_vector(3 downto 0) := "0110";
  signal k : integer := 0;
begin
  -- Signal parameters: read, a deposit and transactions on the caller's driver.
  sig : process
  begin
    wait for 1 ns;
    drive(s, o, k);
    wait for 0 ns;
    assert o = "1111" report "deposit " & to_string(o) severity failure;
    wait for 1 ns;
    assert o = "1001" and k = 0 report "driven " & to_string(o) severity failure;
    wait for 1 ns;
    assert k = 4 report "transport" severity failure;
    report "signals ok";
    wait;
  end process;
  process
    variable q, r, n : integer;
    variable v : std_logic_vector(3 downto 0) := "1100";
    variable rec : pt;
    variable ptr : int_ptr := new integer'(41);
    variable null_ptr : int_ptr;
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
    assert rev("1101") = "1011" report "rev" severity failure;
    assert rev("1101")'left = 1 and rev("1101")'ascending report "rev range" severity failure;
    assert iota(3) = (0, 10, 20) report "iota" severity failure;
    assert iota(0)'length = 0 report "iota null" severity failure;
    assert word(3) = "abc" report "word" severity failure;
    assert nibble(5) = "0101" and nibble(5)'left = 3 report "nibble" severity failure;
    assert norm((3, true, "1010")) = 3 + 1 + 2 report "norm" severity failure;
    flip((3, true, "1010"), rec);
    assert rec = (-3, false, "0101") report "flip" severity failure;
    peek(ptr, n);
    assert n = 41 and ptr.all = 42 report "peek" severity failure;
    peek(null_ptr, n);
    assert n = -1 report "peek null" severity failure;
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
entity nosize is end;
architecture a of nosize is begin
  process begin report integer'image(unsized(1)'length); wait; end process;
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
