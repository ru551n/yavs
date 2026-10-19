-- LRM 10.6.2.1: an element association of an aggregate target that is itself an aggregate target (array of arrays).
entity top is end entity;
architecture a of top is
  type row is array (0 to 1) of integer;
  type mat is array (0 to 1) of row;
  function h return mat is begin return ((1, 2), (3, 4)); end function;
begin
  process
    variable p, q, r, s : integer;
  begin
    ((p, q), (r, s)) := h;
    report "RES p=" & integer'image(p) & " q=" & integer'image(q) & " r=" & integer'image(r) & " s=" & integer'image(s);
    wait;
  end process;
end architecture;
