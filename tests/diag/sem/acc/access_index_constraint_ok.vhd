-- yavs-test: std=2008
-- expect: none
-- LRM 5.4, 6.3: an index constraint on an access type applies to the designated array type
package p is
  type mem_ptr is access bit_vector;
  procedure run;
end package;

package body p is
  procedure run is
    variable v : mem_ptr(0 to 3) := new bit_vector'("1111");
    variable s : mem_ptr;
  begin
    assert v.all(0) = '1' report "element" severity failure;
    v.all(1) := '0';
    for k in v.all'range loop
      null;
    end loop;
    s := new bit_vector(2 to 5);
  end procedure;
end package body;
