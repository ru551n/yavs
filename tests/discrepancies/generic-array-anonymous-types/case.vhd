-- A 2019 generic array type with anonymous index and element types (`type is ...`).
package g is
  generic (type vt is array (type is range <>) of type is private);
  function first_of (v : vt) return vt'element;
end package;
package body g is
  function first_of (v : vt) return vt'element is begin return v(v'left); end function;
end package body;
package iv is new work.g generic map (vt => integer_vector);
entity top is end;
architecture a of top is
begin
  process begin
    report "RES " & integer'image(work.iv.first_of((7, 8, 9)));
    wait;
  end process;
end architecture;
