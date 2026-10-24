-- difftest: top=sigrec_arrays_null std=2008 stop=10ns
-- Null arrays of records (#677): a signal with no element, a null slice and a record with a null
-- array of records as a field are read and driven without a transaction and without an error.
library ieee; use ieee.std_logic_1164.all;
package sigrec_null_pkg is
  type cell is record
    p : std_logic;
    v : std_logic_vector(2 downto 0);
    n : integer;
  end record;
  type cells is array (natural range <>) of cell;
  type holder is record
    tag : std_logic;
    none : cells(1 to 0);
    one : cell;
  end record;
  function img (a : cells) return string;
end package;
package body sigrec_null_pkg is
  function img (a : cells) return string is
  begin
    if a'length = 0 then
      return "empty";
    end if;
    return std_logic'image(a(a'left).p) & integer'image(a'length);
  end;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.sigrec_null_pkg.all;
entity sigrec_arrays_null is end;
architecture a of sigrec_arrays_null is
  signal z : cells(1 to 0);
  signal s : cells(0 to 3);
  signal h : holder;
begin
  p : process
    variable vz : cells(1 to 0);
    variable vs : cells(0 to 3);
    variable vh : holder;
  begin
    wait for 1 ns;
    vz := z;
    z <= vz;
    s(2 to 1) <= vz;
    vs := s;
    report "z=" & img(vz) & " s=" & img(s(1 to 0)) & " " & img(vs);
    vh := h;
    h <= vh;
    h.none <= vz;
    wait for 1 ns;
    vh.one.n := 5;
    vh.one.p := '1';
    h <= vh;
    wait for 1 ns;
    report "h=" & std_logic'image(h.one.p) & integer'image(h.one.n) & " " & img(h.none);
    wait;
  end process;
end;
