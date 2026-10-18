-- yavs-test: std=2008
-- expect: none
-- LRM 12.3, 5.6.3 (OSVVM RandomPkg: package-body constants used inside the protected body)
package pk is
  type pt is protected
    impure function get return integer;
  end protected;
end package;
package body pk is
  constant hidden : integer := 7;
  type pt is protected body
    impure function get return integer is
    begin
      return hidden;
    end function;
  end protected body;
end package body;
