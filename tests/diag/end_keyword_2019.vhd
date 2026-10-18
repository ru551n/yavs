-- yavs: --std 2019
-- VHDL-2019 makes only `end component` optional (LCS-2016-055a).
package p is
  component c is
    port (a : bit);
  end;
  type r is record
    a : bit;
  end;
end package;
