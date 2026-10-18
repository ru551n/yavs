-- difftest: top=generic_package_prot std=2019
package gp is
  generic (type t; init : t);
  type box is protected
    procedure put(v : t);
    impure function get return t;
  end protected;
end package;
package body gp is
  type box is protected body
    variable val : t := init;
    procedure put(v : t) is begin val := v; end;
    impure function get return t is begin return val; end;
  end protected body;
end package body;
package int_gp is new work.gp generic map (t => integer, init => 42);
use work.int_gp.all;
entity generic_package_prot is end;
architecture t of generic_package_prot is
begin
  process
    variable b : box;
  begin
    report "init=" & integer'image(b.get);
    b.put(7);
    report "now=" & integer'image(b.get);
    wait;
  end process;
end;
