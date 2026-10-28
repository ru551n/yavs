-- LRM 5.3.2.2: the index constraint of a record element subtype must lie inside the index
-- subtype of the array type, as any index constraint: string(x'reverse_range) with x'range
-- 0 to 3 has the bound 0 outside POSITIVE, so the declaration of r fails (#637).
entity top is end entity;
architecture a of top is
  procedure foo(x : bit_vector) is
    type rec is record
      f : string(x'reverse_range);
    end record;
    variable r : rec;
  begin
    report "RES declared, length " & integer'image(r.f'length);
  end procedure;
begin
  process
    variable bad : bit_vector(0 to 3);
  begin
    foo(bad);
    wait;
  end process;
end architecture;
