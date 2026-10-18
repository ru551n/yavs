-- yavs-test: std=2008
-- expect: E1004 4:19 entity `noent` not found; an architecture needs a primary unit
-- LRM 3.3
architecture a of noent is
begin
end architecture;
