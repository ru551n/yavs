-- yavs-test: std=2008
-- expect: none
-- LRM 6.7, 7.2, 8.4, 8.5: the value of a user attribute of array type can be indexed or sliced
package p is
  type mem is array (positive range <>) of bit;
  attribute xa : mem;
  attribute xa of mem : type is ('1', '0', '1');
  procedure run;
end package;

package body p is
  procedure run is
    variable b : bit;
    variable s : mem(1 to 2);
  begin
    b := mem'xa(1);
    s := mem'xa(1 to 2);
  end procedure;
end package body;
