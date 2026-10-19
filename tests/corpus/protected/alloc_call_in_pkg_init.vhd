-- difftest: top=c std=2008
-- Access values allocated by package functions that the initial values of a package's
-- shared protected object (declared in the package body) call, used from a process (#246).
package p is
  type mb_t is record size : natural; end record;
  type mb_ptr_t is access mb_t;
  type ptr_array_t is array (natural range <>) of mb_ptr_t;
  type item_t is record id : integer; boxes : ptr_array_t(0 to 1); end record;
  type item_ptr_t is access item_t;
  impure function create(size : natural) return mb_ptr_t;
  type pt is protected
    impure function size(n : natural) return natural;
  end protected;
  impure function size(n : natural) return natural;
end package;
package body p is
  impure function create(size : natural) return mb_ptr_t is
  begin
    return new mb_t'(size => size);
  end function;
  impure function make_item(id : integer) return item_ptr_t is
  begin
    return new item_t'(id => id, boxes => (create(id), create(id * 2)));
  end function;
  type pt is protected body
    variable item : item_ptr_t := make_item(3);
    impure function size(n : natural) return natural is
    begin
      return item.boxes(n).size;
    end;
  end protected body;
  shared variable sv : pt;
  impure function size(n : natural) return natural is
  begin
    return sv.size(n);
  end function;
end package body;
use work.p.all;
entity c is end;
architecture t of c is
begin
  process begin
    report integer'image(size(0)) & " " & integer'image(size(1));
    assert size(0) = 3 and size(1) = 6 severity failure;
    wait;
  end process;
end;
