-- yavs-test: std=2019
-- expect: E1907 22:9 a variable port of a composite of protected types is not supported
-- expect: E1905 30:10 a signal cannot be of a type with protected subelements
-- expect: E1904 31:19 a shared variable of a non-protected type
-- expect: E1905 37:5 a type with protected subelements cannot be assigned
-- expect: E1905 38:5 a record with a protected element cannot be assigned
-- expect: E1905 39:8 no predefined `=` for a type with protected subelements
-- expect: E1905 40:5 an element of a protected type cannot be assigned
-- LRM 5.6.1, 6.4.2.4, 10.6.2.1; LCS-2016-014: composites of protected types are not values
package p is
  type pt is protected
    procedure inc;
  end protected;
  type pa is array (0 to 1) of pt;
  type rec is record a : pt; b : pt; end record;
end package;
package body p is
  type pt is protected body procedure inc is begin end; end protected body;
end package body;
use work.p.all;
entity child is
  port (variable vp : inout pa);
end entity;
architecture a of child is
begin
end architecture;
use work.p.all;
entity e is end entity;
architecture a of e is
  signal s : pa;
  shared variable sv : integer;
begin
  process
    variable v, w : pa;
    variable r : rec;
  begin
    v := w;
    r := r;
    if v = w then end if;
    v(0) := w(0);
    wait;
  end process;
end architecture;
