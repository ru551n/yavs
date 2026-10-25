-- yavs-test: std=2008
-- expect: none
-- LRM 7.2, 9.3.3.3: a user attribute of a subtype is an expression of the index type as a choice
package p is
  subtype s10 is string (1 to 4);
  attribute attr1 : integer;
  attribute attr1 of s10 : subtype is 4;
  procedure run;
end package;

package body p is
  procedure run is
    variable v : s10;
  begin
    v := (1 | s10'attr1 => 'a', others => 'b');
  end procedure;
end package body;
