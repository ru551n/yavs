-- yavs-test: std=2019
-- expect: none
-- LRM 5.4.1 (2019): access types may designate protected and file types
package p is
  type pt is protected
    procedure set (v : integer);
  end protected;
  type pt_ptr is access pt;
  type ft is file of integer;
  type ft_ptr is access ft;
end package;

package body p is
  type pt is protected body
    variable x : integer := 0;
    procedure set (v : integer) is
    begin
      x := v;
    end procedure;
  end protected body;

  impure function make return pt_ptr is
  begin
    return new pt;
  end function;
end package body;
