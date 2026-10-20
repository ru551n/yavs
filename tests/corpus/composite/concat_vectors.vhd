-- difftest: top=concat_vectors std=2008
-- `&` of one-dimensional arrays of scalars and elements (the one-instruction form): logic,
-- bit, boolean, character and integer arrays, ascending and descending index subtypes, null
-- operands, slices of signals (a shift register), more than one block of plane elements,
-- metavalues, three operands, and a result in an array type with its own index subtype.
library ieee;
use ieee.std_logic_1164.all;
entity concat_vectors is end;
architecture t of concat_vectors is
  type iv is array (natural range <>) of integer;
  type bv is array (integer range <>) of boolean;
  subtype didx is integer range 9 downto 0;
  type dv is array (didx range <>) of std_logic;
  subtype w4 is std_logic_vector(3 downto 0);
  signal sr : std_logic_vector(15 downto 0) := x"ACE1";
  signal sb : bit_vector(0 to 7) := "10110001";
  signal clk : std_logic := '0';
  function img(v : iv) return string is
  begin
    if v'length = 0 then return "-"; end if;
    return integer'image(v(v'left)) & "," & img(v(v'left + 1 to v'right));
  end;
  function img(v : bv) return string is
  begin
    if v'length = 0 then return "-"; end if;
    return boolean'image(v(v'left)) & "," & img(v(v'left + 1 to v'right));
  end;
begin
  process
    variable a : std_logic_vector(3 downto 0) := "10XZ";
    variable b : std_logic_vector(0 to 2) := "UW-";
    variable e : std_logic_vector(0 to -1);
    variable big : std_logic_vector(99 downto 0) := (others => '1');
    variable big2 : std_logic_vector(0 to 69) := (others => 'L');
    variable r : std_logic_vector(104 downto 0);
    variable s : std_logic_vector(31 downto 0);
    variable w7 : std_logic_vector(6 downto 0);
    variable bb : bit_vector(1 to 5);
    variable i1 : iv(1 to 2) := (10, 20);
    variable i2 : iv(5 to 7) := (30, 40, 50);
    variable ie : iv(1 to 0);
    variable bo : bv(0 to 1) := (true, false);
    variable ch : string(1 to 3) := "abc";
    variable str6 : string(1 to 6);
    variable d : dv(9 downto 0);
    variable q : dv(9 downto 6);
    variable qe : dv(1 to 0);
    variable q5 : dv(9 downto 5);
  begin
    report to_string(a & b) & " " & to_string(b & a) & " " & to_string(a & '1') & " "
      & to_string('0' & a) & " " & to_string(std_logic_vector'('H' & 'Z'));
    w7 := a & b;
    report to_string(e & a) & "|" & to_string(a & e) & "|"
      & integer'image(w7'length) & " " & integer'image(w7'left) & " " & integer'image(w7'right);
    r := big & big2(0 to 4);
    report to_string(r(104 downto 94)) & " " & to_string(r(5 downto 0));
    r := '1' & big & "0000";
    report to_string(r(104 downto 98)) & " " & to_string(r(7 downto 0));
    s := big(30 downto 0) & '0';
    report to_string(s);
    s := a & a & a & a & a & a & a & a;
    report to_string(s);
    s := sr & sr;
    report to_string(s);
    s := (sr(14 downto 0) & (sr(15) xor sr(13))) & sr;
    report to_string(s);
    bb := sb(2 to 5) & '1';
    report to_string(bb) & " " & to_string(bit_vector'("11" & sb(0 to 2)));
    report img(i1 & i2) & " " & img(i1 & 5) & " " & img(7 & i2) & " " & img(ie & i1) & " "
      & img(i2 & ie) & " " & img(i1 & i2 & i1);
    report img(bo & bo) & " " & img(bo & true) & " " & img(false & bo);
    report ch & "x" & ch & 'y' & 'z' & "";
    str6 := ch & ch;
    report integer'image(str6'left) & integer'image(str6'right);
    d := "1010101010";
    q := d(9 downto 6) & qe;
    report to_string(q);
    q5 := (d(1) & d(0)) & q(8 downto 6);
    report to_string(q5) & " " & integer'image(q5'left);
    q5 := qe & q5;
    report to_string(q5);
    wait for 1 ns;
    sr <= sr(14 downto 0) & (sr(15) xor sr(13) xor sr(12) xor sr(10));
    wait for 1 ns;
    report to_string(sr);
    sr <= sr(0) & sr(15 downto 1);
    wait for 1 ns;
    report to_string(sr);
    wait;
  end process;
end;
