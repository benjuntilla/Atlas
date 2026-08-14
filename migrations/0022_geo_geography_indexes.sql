-- Geography-cast GIST indexes.
--
-- The geo-engine queries compare distances in METERS, which on a 4326
-- column requires casting both operands to `geography` (a bare geometry
-- comparison is in degrees — see queries/locations.rs for the full note).
--
-- The existing indexes are built on the raw geometry columns:
--     idx_geo_locations_position   ON geo.locations   USING GIST(position)
--     idx_geo_geofences_center     ON geo.geofences   USING GIST(center)
--     idx_safety_ratings_geom      ON geo.safety_ratings USING GIST(segment_geom)
--
-- The planner cannot use a geometry index for a `::geography` predicate,
-- so without these expression indexes every nearby / geofence-check /
-- route-score query degrades to a sequential scan. That is survivable at
-- dev-seed volumes and very much not at production ones.
--
-- The geometry->geography cast is IMMUTABLE for SRID 4326, which is what
-- makes indexing the expression legal.
--
-- The geometry indexes are kept: they still serve any bounding-box or
-- containment predicate that stays in geometry space.

CREATE INDEX idx_geo_locations_position_geog
    ON geo.locations USING GIST ((position::geography));

CREATE INDEX idx_geo_geofences_center_geog
    ON geo.geofences USING GIST ((center::geography));

CREATE INDEX idx_safety_ratings_geom_geog
    ON geo.safety_ratings USING GIST ((segment_geom::geography));
