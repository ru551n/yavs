-- A 2019 generic array type with an index constraint: `array (0 to 3) of type is (<>)`.
package g is
  generic (type ct is array (0 to 3) of type is (<>));
  function second (c : ct) return ct'element;
end package;
package body g is
  function second (c : ct) return ct'element is begin return c(1); end function;
end package body;
package u is type quad is array (0 to 3) of character; end package;
package qi is new work.g generic map (ct => work.u.quad);
entity top is end;
architecture a of top is
begin
  process begin
    report "RES " & character'image(work.qi.second("abcd"));
    wait;
  end process;
end architecture;
