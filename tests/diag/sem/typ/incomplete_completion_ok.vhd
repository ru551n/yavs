-- yavs-test: std=2008
-- expect: none
-- LRM 5.4.2
package p is
  type e;
  type ep is access e;
  type e is (red, green);
  constant c : e := green;
  type arr;
  type ap is access arr;
  type arr is array (0 to 3) of bit;
  type node;
  type node_ptr is access node;
  type node_ptr_ptr is access node_ptr;
  type node is record
    next_n : node_ptr;
    v      : integer;
  end record;
  procedure run;
end package;

package body p is
  procedure run is
    variable a : ap;
    variable x : ep;
    variable n : node_ptr;
    variable m : node_ptr_ptr;
  begin
    a := new arr;
    a(3) := '1';
    x := new e'(red);
    n := new node'(next_n => null, v => 1);
    n.next_n := new node;
    n.next_n.v := n.v + a'length;
    m := new node_ptr'(n);
    m.all.all.v := 2;
  end procedure;
end package body;
