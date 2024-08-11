#![doc = include_str!("../README.md")]
// The contents of this file were primarily ported from Agent.cc from RVO2-3D
// with significant alterations. As per the Apache-2.0 license, the original
// copyright notice has been included, excluding those notices that do not
// pertain to the derivate work:
//
// Agent.cc
// RVO2 Library
//
// SPDX-FileCopyrightText: 2008 University of North Carolina at Chapel Hill
//
// The authors may be contacted via:
//
// Jur van den Berg, Stephen J. Guy, Jamie Snape, Ming C. Lin, Dinesh Manocha
// Dept. of Computer Science
// 201 S. Columbia St.
// Frederick P. Brooks, Jr. Computer Science Bldg.
// Chapel Hill, N.C. 27599-3175
// United States of America
//
// <https://gamma.cs.unc.edu/RVO2/>
mod linear_programming;
mod simulator;

use std::borrow::Cow;

use crate::linear_programming::{solve_linear_program, Plane};

pub use glam::Vec3;
pub use simulator::{AgentParameters, Simulator, SimulatorMargin};

/// A single agent in the simulation.
#[derive(Clone, PartialEq, Debug)]
pub struct Agent {
  /// The position of the agent.
  pub position: Vec3,
  /// The current velocity of the agent.
  pub velocity: Vec3,

  /// The radius of the agent. Agents will use this to avoid bumping into each
  /// other.
  pub radius: f32,
  /// The amount of responsibility an agent has to avoid other agents. The
  /// amount of avoidance between two agents is then dependent on the ratio of
  /// the responsibility between the agents. Note this does not affect
  /// avoidance of obstacles.
  pub avoidance_responsibility: f32,
}

/// Parameters for computing the avoidance vector.
#[derive(Clone, PartialEq, Debug)]
pub struct AvoidanceOptions {
  /// How long in the future should collisions be considered between agents.
  pub time_horizon: f32,
}

impl Agent {
  /// Computes a velocity based off the agent's preferred velocity (usually the
  /// direction to its current goal/waypoint). This new velocity is intended to
  /// avoid running into the agent's `neighbours`. This is not always possible,
  /// but agents will attempt to resolve any collisions in a reasonable fashion.
  /// The `max_speed` is the maximum magnitude of the returned velocity. Even if
  /// the `preferred_velocity` is larger than `max_speed`, the resulting vector
  /// will be at most `max_speed` in length. The `time_step` helps determine the
  /// velocity in cases of existing collisions, and must be positive.
  pub fn compute_avoiding_velocity(
    &self,
    neighbours: &[Cow<'_, Agent>],
    preferred_velocity: Vec3,
    max_speed: f32,
    time_step: f32,
    avoidance_options: &AvoidanceOptions,
  ) -> Vec3 {
    assert!(time_step > 0.0, "time_step must be positive, was {}", time_step);

    let planes = neighbours
      .iter()
      .map(|neighbour| {
        self.get_plane_for_neighbour(
          neighbour,
          avoidance_options.time_horizon,
          time_step,
        )
      })
      .collect::<Vec<Plane>>();

    solve_linear_program(&planes, max_speed, preferred_velocity)
  }

  /// Creates a plane to describe the half-space of valid velocities that should
  /// not collide with `neighbour`.
  fn get_plane_for_neighbour(
    &self,
    neighbour: &Agent,
    time_horizon: f32,
    time_step: f32,
  ) -> Plane {
    // There are two parts to the velocity obstacle induced by `neighbour`.
    // 1) The cut-off sphere. This is where the agent collides with `neighbour`
    // after some time (either `time_horizon` or `time_step`).
    // 2) The cut-off shadow. Any velocity that is just scaled up from a
    // velocity in the cut-off sphere will also hit `neighbour`.
    //
    // If the relative position and velocity is used, the cut-off for the shadow
    // will be directed toward the origin.

    let relative_neighbour_position = neighbour.position - self.position;
    let relative_agent_velocity = self.velocity - neighbour.velocity;

    let distance_squared = relative_neighbour_position.length_squared();

    let sum_radius = self.radius + neighbour.radius;
    let sum_radius_squared = sum_radius * sum_radius;

    let vo_normal;
    let relative_velocity_projected_to_vo;
    let inside_vo;

    // Find out if the agent is inside the cut-off sphere. Note: since both the
    // distance to the cut-off sphere and the radius of the cut-off sphere is
    // scaled by `time_horizon` (or `time_step` depending on the situation),
    // factoring out those terms and cancelling yields this simpler expression.
    if distance_squared > sum_radius_squared {
      // No collision, so either project on to the cut-off sphere, or the
      // cut-off shadow.
      //
      // The edges of the cut-off shadow lies along the tangents of the sphere
      // that intersects the origin (since the tangents are the planes that just
      // graze the cut-off sphere and so these planes divide the "shadowed"
      // velocities from the "unshadowed" velocities).
      //
      // Since the shadows are caused by the tangent "ring", velocities should
      // be projected to the cut-off sphere when they are on one-side of
      // the tangent ring, and should be projected to the shadow when on
      // the other-side of the tangent ring.

      let cutoff_sphere_center = relative_neighbour_position / time_horizon;
      let cutoff_sphere_center_to_relative_velocity =
        relative_agent_velocity - cutoff_sphere_center;
      let cutoff_sphere_center_to_relative_velocity_length_squared =
        cutoff_sphere_center_to_relative_velocity.length_squared();

      let dot = cutoff_sphere_center_to_relative_velocity
        .dot(relative_neighbour_position);

      // TODO: Figure out why this works.
      if dot < 0.0
        && dot * dot
          > sum_radius_squared
            * cutoff_sphere_center_to_relative_velocity_length_squared
      {
        // The relative velocity has not gone past the cut-off sphere tangent
        // ring yet, so project onto the cut-off sphere.

        let cutoff_sphere_radius = sum_radius / time_horizon;

        vo_normal =
          cutoff_sphere_center_to_relative_velocity.normalize_or_zero();
        relative_velocity_projected_to_vo =
          vo_normal * cutoff_sphere_radius + cutoff_sphere_center;
        inside_vo = cutoff_sphere_center_to_relative_velocity_length_squared
          < cutoff_sphere_radius * cutoff_sphere_radius;
      } else {
        // The relative velocity is past the cut-off sphere tangent ring, so
        // project onto the shadow (which is a cone). Note this means we can
        // ignore the time_horizon, since the cone is the same regardless of the
        // time horizon.

        // We want to find the normal of the tangent plane. As a simplification,
        // we will only consider the plane intersecting zero and the
        // relative_agent_velocity (since the normal of the tangent plane
        // must be in that plane). Consider a ray passing through
        // relative_agent_velocity and in the direction of
        // relative_neighbour_position, and another ray passing through the
        // origin and in the direction of relative_neighbour_position. This
        // gives us two parallel lines, and the tangent plane creates a
        // transversal across them. Using angle rules, we can find that the
        // triangle between the cutoff sphere center, the origin, and the
        // tangent plane is a similar triangle to the triangle between the
        // distance between the rays, the intersection point of the tangent
        // plane's normal and the relative_agent_velocity ray, and the
        // projection of that point onto the relative_neighbour_position ray.
        let tangent_ring_triangle_leg_squared =
          distance_squared - sum_radius_squared;

        let squared_distance_between_rays = relative_neighbour_position
          .cross(relative_agent_velocity)
          .length_squared();

        // Use the Pythagorean theorem to solve for the time when the
        // relative_agent_velocity ray has the "correct" distance from the
        // origin for where the intersection point should be. The equation is
        // roughly:
        // (relative_agent_velocity + t * relative_neighbour_position) ^ 2
        //   = squared_distance_between_rays / tangent_ring_triangle_leg_squared

        let a = relative_neighbour_position.length_squared();
        // Note: we *should* multiply this by 2, but we can actually just factor
        // the two out to skip a few multiplications.
        let b = relative_neighbour_position.dot(relative_agent_velocity);
        let c = relative_agent_velocity.length_squared()
          - squared_distance_between_rays / tangent_ring_triangle_leg_squared;
        // Always choose the negative solution, since we know the intersection
        // point must be behind us (since if it was ahead, we should have
        // projected to the cutoff sphere instead).
        let t = (-b - (b * b - a * c).sqrt()) / a;

        vo_normal = (relative_agent_velocity + t * relative_neighbour_position)
          .normalize_or_zero();
        let distance_to_plane = Plane { normal: vo_normal, point: Vec3::ZERO }
          .signed_distance_to_plane(relative_agent_velocity);
        inside_vo = distance_to_plane < 0.0;
        relative_velocity_projected_to_vo =
          relative_agent_velocity - distance_to_plane * vo_normal;
      }
    } else {
      // Collision. Project on cut-off sphere at time `time_step`.

      // Find the velocity such that after `time_step` the agent would be at the
      // neighbours position.
      let cutoff_sphere_center = relative_neighbour_position / time_step;
      let cutoff_sphere_radius = sum_radius / time_step;

      // The direction of the velocity from `cutoff_sphere_center` is therefore
      // the normal to the velocity obstacle.
      vo_normal = {
        let velocity_from_circle_center =
          relative_agent_velocity - cutoff_sphere_center;
        // If the vector has a length of zero, pick a random direction. Fork the
        // implementation of `normalize_or` so we only compute random
        // values if necessary (which should be very rare).
        let recip = velocity_from_circle_center.length_recip();
        if recip.is_finite() && recip > 0.0 {
          velocity_from_circle_center * recip
        } else {
          // Generate uniform random point based on
          // https://math.stackexchange.com/a/1586015
          let z: f32 = rand::random();
          let longitude: f32 = rand::random();

          let z_normalize = (1.0 - z * z).sqrt();
          Vec3::new(
            longitude.cos() * z_normalize,
            longitude.sin() * z_normalize,
            z,
          )
        }
      };
      // Get the point on the cut-off sphere in that direction (which is the
      // agent's velocity projected to the sphere).
      relative_velocity_projected_to_vo =
        vo_normal * cutoff_sphere_radius + cutoff_sphere_center;
      inside_vo = true;
    }

    // As in the paper, `u` is the vector from the relative velocity to the
    // nearest point outside the velocity obstacle.
    let u = relative_velocity_projected_to_vo - relative_agent_velocity;

    let responsibility = if inside_vo {
      self.avoidance_responsibility
        / (self.avoidance_responsibility + neighbour.avoidance_responsibility)
    } else {
      1.0
    };

    Plane { point: self.velocity + u * responsibility, normal: vo_normal }
  }
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod test;
