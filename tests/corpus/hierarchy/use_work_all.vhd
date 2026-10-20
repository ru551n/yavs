-- difftest: top=use_work_all std=2008
-- `use work.all` in front of a unit: names that are no unit of the library still resolve
-- to std.standard (LRM 12.4).
use work.all;
entity use_work_all is end entity;
architecture a of use_work_all is
  function weird (bw : integer range 2 to 32) return integer is
  begin
    return -(2 ** (bw - 1));
  end function;
begin
  process begin
    report integer'image(weird(10)) & " " & boolean'image(true);
    wait;
  end process;
end architecture;
