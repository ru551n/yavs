-- LRM 8.7: a relative path in a subprogram starts at the innermost concurrent region of each
-- elaborated copy: every generate iteration reads its own instance.
entity leaf is
  generic (id : integer);
end;
architecture a of leaf is
  constant tag : integer := id * 10;
begin
end;
entity top is end;
architecture t of top is
begin
  g : for i in 1 to 2 generate
    u : entity work.leaf generic map (id => i);
    b : block
      impure function tagof return integer is
      begin
        return <<constant ^.u.tag : integer>>;
      end function;
    begin
      process
      begin
        report "RES " & integer'image(i) & " " & integer'image(tagof);
        wait;
      end process;
    end block;
  end generate;
end;
