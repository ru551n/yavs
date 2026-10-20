-- difftest: top=tb_package_body_item_name std=2008
-- A package body can name an item of its package that has the package's name.
package freq_pbi is
  type freq_pbi is range 0 to 100;
  function pos_of (x : freq_pbi) return integer;
end package;
package body freq_pbi is
  function pos_of (x : freq_pbi) return integer is
  begin
    return freq_pbi'pos(x);
  end function;
end package body;

entity tb_package_body_item_name is end;
architecture t of tb_package_body_item_name is
begin
  process begin
    report integer'image(work.freq_pbi.pos_of(5));
    wait;
  end process;
end;
