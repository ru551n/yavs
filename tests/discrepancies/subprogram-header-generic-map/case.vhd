-- LRM 4.2.1, 4.4: a subprogram header with a generic list and a generic map.
entity top is end entity;
architecture a of top is
  function f generic (type t) generic map (t => integer) parameter (v : t) return t is
  begin return v; end function;
begin
  process begin report "RES " & integer'image(f(3)); wait; end process;
end architecture;
