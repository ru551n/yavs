-- LRM 2019 16.2.2, 5.7: 'IMAGE and TO_STRING of composites; how character literal elements
-- and quotes inside string elements are written.
entity top is end entity;
architecture a of top is
  type rec_t is record x : integer; y : bit; end record;
  type mixed_t is (red, 'x', green);
  type mixed_vec_t is array (natural range <>) of mixed_t;
  constant r : rec_t := (3, '1');
  constant m : mixed_vec_t := (red, 'x');
  constant q : string := "a""b";
begin
  process
  begin
    report "RES " & to_string(r);
    report "RES " & rec_t'image(r);
    report "RES " & mixed_vec_t'image(m);
    report "RES " & string'image(q);
    wait;
  end process;
end architecture;
