-- LRM 10.6.2.1, 9.3.3.3: an aggregate target of a multi-dimensional array, an aggregate of aggregates.
entity top is end entity;
architecture a of top is
  type m2 is array (0 to 1, 0 to 1) of integer;
  function k return m2 is begin return ((1, 2), (3, 4)); end function;
begin
  process
    variable p, q, r, s : integer;
  begin
    ((p, q), (r, s)) := k;
    report "RES p=" & integer'image(p) & " q=" & integer'image(q) & " r=" & integer'image(r) & " s=" & integer'image(s);
    wait;
  end process;
end architecture;
