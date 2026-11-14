#include <netproto/802_11/ieee80211_var.h>
#ifndef _ATHP_DFLY_80211_EXTRA_
#define	_ATHP_DFLY_80211_EXTRA_
#define	IEEE80211_IS_MGMT(wh)						\
	(((wh)->i_fc[0] & IEEE80211_FC0_TYPE_MASK) == IEEE80211_FC0_TYPE_MGT)
#define	IEEE80211_IS_QOS_ANY(wh)					\
	((((wh)->i_fc[0] & IEEE80211_FC0_TYPE_MASK) == IEEE80211_FC0_TYPE_DATA) && \
	 ((wh)->i_fc[0] & IEEE80211_FC0_SUBTYPE_QOS))
static __inline uint32_t
ieee80211_get_channel_center_freq(const struct ieee80211_channel *c)
{
	return (c->ic_freq);
}
static __inline uint32_t
ieee80211_get_channel_center_freq1(const struct ieee80211_channel *c)
{
	if (IEEE80211_IS_CHAN_HT40U(c))
		return (c->ic_freq + 10);
	if (IEEE80211_IS_CHAN_HT40D(c))
		return (c->ic_freq - 10);
	return (c->ic_freq);
}
static __inline uint32_t
ieee80211_get_channel_center_freq2(const struct ieee80211_channel *c)
{
	return (0);
}
#define	ieee80211_output_seqno_assign(ni, tid, m)	\
	athp_dfly_seqno_assign((ni), (m))
void	athp_dfly_seqno_assign(struct ieee80211_node *, struct mbuf *);
#endif
#ifndef _ATHP_DFLY_80211_EXTRA2_
#define	_ATHP_DFLY_80211_EXTRA2_
/* No VHT or U-APSD in DragonFly's net80211: those paths compile to nothing. */
#define	IEEE80211_NODE_VHT		0
#define	IEEE80211_NODE_UAPSD		0
#define	IEEE80211_FEXT_VHT		0
#define	IEEE80211_FEXT_UAPSD		0
#define	IEEE80211_C_UAPSD		0
#define	IEEE80211_FEXT_FRAG_OFFLOAD	0
#define	IEEE80211_FEXT_SEQNO_OFFLOAD	0
#define	ni_uapsd			ni_flags
#define	IEEE80211_QOS_CTL_LEN		2
#define	IEEE80211_FC0_SUBTYPE_QOS_MASK_ANY IEEE80211_FC0_SUBTYPE_QOS
#define	IEEE80211_IS_PROTECTED(wh)	((wh)->i_fc[1] & IEEE80211_FC1_PROTECTED)
#define	NET80211_CBW_FLAG_HT40		0x01
#define	NET80211_CBW_FLAG_VHT80		0x02
#define	IEEE80211_VAP_GET_SLOTTIME(vap)					\
	(((vap)->iv_ic->ic_flags & IEEE80211_F_SHSLOT) ?		\
	    IEEE80211_DUR_SHSLOT : IEEE80211_DUR_SLOT)
#define	ieee80211_ampdu_rx_stop_ext(ni, tid)				\
	((ni)->ni_ic->ic_ampdu_rx_stop((ni), &(ni)->ni_rx_ampdu[(tid)]))
/* the rx stats ride along as an mbuf tag (ieee80211_add_rx_params) */
#define	ieee80211_input_mimo(ni, m)	(ieee80211_input_mimo)((ni), (m), NULL)
#define	ieee80211_input_mimo_all(ic, m)	(ieee80211_input_mimo_all)((ic), (m), NULL)
#endif
