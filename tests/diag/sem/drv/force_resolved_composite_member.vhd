-- yavs-test: std=2008
-- expect: E1362 21:5 the target of a force or release is a member of a composite signal resolved as a whole
-- expect: E1362 22:5 the target of a force or release is a member of a composite signal resolved as a whole
-- LRM 10.5.2.1: the whole composite may be forced (`s <= force ...` is legal).
package p is
  type rec_u is record a : integer; n : integer; end record;
  type rec_drv is array (natural range <>) of rec_u;
  function first(v : rec_drv) return rec_u;
  subtype rec_t is first rec_u;
  type arr_u is array (0 to 3) of bit;
  type arr_drv is array (natural range <>) of arr_u;
  function first(v : arr_drv) return arr_u;
  subtype arr_t is first arr_u;
end package;
use work.p.all;
entity e is end entity;
architecture a of e is
  signal s : rec_t; signal v : arr_t;
begin
  process begin
    s.n <= force 1;
    v(1 to 2) <= release;
    s <= force (1, 2);
    v <= release;
    wait;
  end process;
end architecture;
