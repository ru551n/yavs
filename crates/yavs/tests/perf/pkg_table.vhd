-- A function called while elaborating reads a package constant built by a function: the table is
-- built once, in the package state, not again for every call.
package pkg_table_pkg is
  type iv is array (natural range <>) of integer;
  function pick (i : integer) return integer;
end;
package body pkg_table_pkg is
  function mk (n : integer) return iv is
    variable r : iv(0 to n - 1);
  begin
    for i in r'range loop r(i) := i * i + 1; end loop;
    return r;
  end;
  constant T : iv := mk(64);
  function pick (i : integer) return integer is begin return T(i); end;
end;
use work.pkg_table_pkg.all;
entity pkg_table is end;
architecture a of pkg_table is
  constant c1 : integer := pick(1);
  constant c2 : integer := pick(2);
  constant c3 : integer := pick(3);
  constant c4 : integer := pick(4);
  constant c5 : integer := pick(5);
  constant c6 : integer := pick(6);
  constant c7 : integer := pick(7);
  constant c8 : integer := pick(8);
  signal s1 : bit_vector(1 to c1);
  signal s2 : bit_vector(1 to c2);
  signal s3 : bit_vector(1 to c3);
  signal s4 : bit_vector(1 to c4);
  signal s5 : bit_vector(1 to c5);
  signal s6 : bit_vector(1 to c6);
  signal s7 : bit_vector(1 to c7);
  signal s8 : bit_vector(1 to c8);
begin
  process begin
    report "sum=" & integer'image(s1'length + s2'length + s3'length + s4'length + s5'length + s6'length + s7'length + s8'length);
    wait;
  end process;
end;
