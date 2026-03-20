# Networking Hardening Checklist

- [ ] Create a dedicated ECS security group for `dev`
- [ ] Create a dedicated ECS security group for `staging`
- [ ] Create a dedicated ECS security group for `prod`
- [ ] Stop sharing one security group across all environments

- [ ] Verify each environment security group has only the egress rules the worker actually needs
- [ ] Avoid broad temporary ingress or egress rules becoming shared prod policy
- [ ] Tag security groups clearly by environment and service

- [ ] Create dedicated subnets for `prod` worker tasks
- [ ] Prefer dedicated subnets for `staging` worker tasks
- [ ] Keep `dev` separate if practical, but this is lower priority than security group separation

- [ ] Confirm route tables and NAT path for each environment are intentional
- [ ] Confirm public IP assignment is intentional for each environment
- [ ] If workers do not need inbound internet access, keep them behind controlled egress only

- [ ] Update GitHub environment vars so `ECS_SUBNET_IDS` is environment-specific
- [ ] Update GitHub environment vars so `ECS_SECURITY_GROUP_IDS` is environment-specific
- [ ] Remove shared repo-level network vars after environment-specific values are in place

- [ ] Verify `dev`, `staging`, and `prod` ECS services each deploy with the intended security groups and subnets
- [ ] Re-run staging happy-path end-to-end validation after network changes
- [ ] Re-run staging retry and DLQ validation after network changes
- [ ] Confirm prod deploy still succeeds with its dedicated network config

## Minimum Bar

- [ ] Separate `prod` security group
- [ ] Environment-scoped GitHub vars for network config
- [ ] No shared prod network policy with `dev` or `staging`
